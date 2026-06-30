//! Spike D — GC integration. Hard numbers for:
//!  (a) write-barrier cost on a table-mutation-heavy loop (isolated + end-to-end),
//!  (b) full-collection cost vs live-heap size (the real per-tick determinism cost in the
//!      "full-collect at instruction-counter boundary" model),
//!  (c) the RETAIN pattern: hold an exported table handle across forced GC cycles, call per
//!      tick, then serde a RETAINED value OUT into a Rust struct OUTSIDE mutate;
//!      stash/fetch cost vs deep-copy cost,
//!  (d) gc-arena vs slotmap: mutation throughput + O(1) snapshot/rollback cost.

use std::hint::black_box;
use std::time::Instant;

use gc_arena::barrier::Unlock;
use gc_arena::lock::RefLock;
use gc_arena::{Arena, Collect, DynamicRoot, DynamicRootSet, Gc, Rootable, Static};
use slotmap::{new_key_type, SlotMap};

// ----- a small deterministic PRNG so inputs are data-dependent (no const-fold/DCE) -----
struct Rng(u64);
impl Rng {
    #[inline]
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    #[inline]
    fn idx(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

// ====================== gc-arena value/heap model ======================
#[derive(Collect, Clone)]
#[collect(no_drop)]
enum GValue<'gc> {
    Nil,
    Num(f64),
    Str(Gc<'gc, Static<String>>),
    Table(Gc<'gc, RefLock<GTable<'gc>>>),
}

#[derive(Collect)]
#[collect(no_drop)]
struct GTable<'gc> {
    arr: Vec<GValue<'gc>>,
}

#[derive(Collect)]
#[collect(no_drop)]
struct GcRoot<'gc> {
    tables: Gc<'gc, RefLock<Vec<Gc<'gc, RefLock<GTable<'gc>>>>>>,
    roots: DynamicRootSet<'gc>,
}
type GcArena = Arena<Rootable![GcRoot<'_>]>;

fn build_gc_arena(n_tables: usize, slots_per: usize) -> GcArena {
    GcArena::new(|mc| {
        let mut v = Vec::with_capacity(n_tables);
        for t in 0..n_tables {
            let mut arr = Vec::with_capacity(slots_per);
            for k in 0..slots_per {
                arr.push(GValue::Num((t * slots_per + k) as f64));
            }
            v.push(Gc::new(mc, RefLock::new(GTable { arr })));
        }
        GcRoot {
            tables: Gc::new(mc, RefLock::new(v)),
            roots: DynamicRootSet::new(mc),
        }
    })
}

// ====================== slotmap value/heap model ======================
new_key_type! { struct STableKey; }
#[derive(Clone)]
enum SValue {
    #[allow(dead_code)]
    Nil,
    Num(f64),
    #[allow(dead_code)]
    Str(u32),
    #[allow(dead_code)]
    Table(STableKey),
}
#[derive(Clone)]
struct STable {
    arr: Vec<SValue>,
}
struct SHeap {
    tables: SlotMap<STableKey, STable>,
    roots: Vec<STableKey>,
}

fn build_slotmap_heap(n_tables: usize, slots_per: usize) -> SHeap {
    let mut tables = SlotMap::with_key();
    let mut roots = Vec::with_capacity(n_tables);
    for t in 0..n_tables {
        let mut arr = Vec::with_capacity(slots_per);
        for k in 0..slots_per {
            arr.push(SValue::Num((t * slots_per + k) as f64));
        }
        roots.push(tables.insert(STable { arr }));
    }
    SHeap { tables, roots }
}

// ====================== (a) write-barrier isolation ======================
fn bench_barrier_isolated(iters: usize) {
    let n_tables = 256usize;
    let slots = 16usize;
    let arena = build_gc_arena(n_tables, slots);
    let phase = format!("{:?}", arena.collection_phase());

    let t0 = Instant::now();
    let acc_b = arena.mutate(|mc, root| {
        let tables = root.tables.borrow();
        let mut rng = Rng(0x1234_5678);
        let mut acc = 0.0f64;
        for _ in 0..iters {
            let ti = rng.idx(n_tables);
            let si = rng.idx(slots);
            let tbl = tables[ti];
            let mut g = tbl.unlock(mc).borrow_mut(); // BARRIER via Gc::write
            if let GValue::Num(x) = &mut g.arr[si] {
                *x += 1.0;
                acc += *x;
            }
        }
        acc
    });
    let dt_b = t0.elapsed();

    let t1 = Instant::now();
    let acc_n = arena.mutate(|_mc, root| {
        let tables = root.tables.borrow();
        let mut rng = Rng(0x1234_5678);
        let mut acc = 0.0f64;
        for _ in 0..iters {
            let ti = rng.idx(n_tables);
            let si = rng.idx(slots);
            let tbl = tables[ti];
            let mut g = unsafe { tbl.unlock_unchecked().borrow_mut() }; // NO barrier
            if let GValue::Num(x) = &mut g.arr[si] {
                *x += 1.0;
                acc += *x;
            }
        }
        acc
    });
    let dt_n = t1.elapsed();

    black_box(acc_b);
    black_box(acc_n);
    let ns_b = dt_b.as_nanos() as f64 / iters as f64;
    let ns_n = dt_n.as_nanos() as f64 / iters as f64;
    println!(
        "[a/barrier-isolated] phase={phase} iters={iters}\n  with-barrier   = {ns_b:.3} ns/mut\n  no-barrier     = {ns_n:.3} ns/mut\n  barrier delta  = {:.3} ns/mut",
        ns_b - ns_n
    );
}

fn bench_barrier_marking(iters: usize) {
    let n_tables = 256usize;
    let slots = 16usize;
    let mut arena = build_gc_arena(n_tables, slots);
    // Add a large debt then run marking to completion -> arena sits in Mark/Marked phase with
    // black objects. Mutating a black table during Mark fires the #[cold] make_gray_again barrier.
    arena.metrics().adjust_debt(1.0e12);
    let _ = arena.mark_debt();
    let phase = format!("{:?}", arena.collection_phase());

    let t0 = Instant::now();
    let acc = arena.mutate(|mc, root| {
        let tables = root.tables.borrow();
        let mut rng = Rng(0xABCD);
        let mut acc = 0.0f64;
        for _ in 0..iters {
            let ti = rng.idx(n_tables);
            let si = rng.idx(slots);
            let tbl = tables[ti];
            let mut g = tbl.unlock(mc).borrow_mut();
            if let GValue::Num(x) = &mut g.arr[si] {
                *x += 1.0;
                acc += *x;
            }
        }
        acc
    });
    let dt = t0.elapsed();
    black_box(acc);
    println!(
        "[a/barrier-marking] phase={phase} iters={iters}  with-barrier = {:.3} ns/mut",
        dt.as_nanos() as f64 / iters as f64
    );
}

// ====================== end-to-end throughput ======================
fn bench_throughput(iters: usize) {
    let n_tables = 256usize;
    let slots = 16usize;

    let arena = build_gc_arena(n_tables, slots);
    let t = Instant::now();
    let a1 = arena.mutate(|mc, root| {
        let tables = root.tables.borrow();
        let mut rng = Rng(7);
        let mut acc = 0.0;
        for _ in 0..iters {
            let ti = rng.idx(n_tables);
            let si = rng.idx(slots);
            let mut g = tables[ti].unlock(mc).borrow_mut();
            if let GValue::Num(x) = &mut g.arr[si] {
                *x += 1.0;
                acc += *x;
            }
        }
        acc
    });
    let gc_ns = t.elapsed().as_nanos() as f64 / iters as f64;

    let mut heap = build_slotmap_heap(n_tables, slots);
    let t = Instant::now();
    let mut rng = Rng(7);
    let mut a2 = 0.0;
    for _ in 0..iters {
        let ti = rng.idx(n_tables);
        let si = rng.idx(slots);
        let key = heap.roots[ti];
        let tbl = &mut heap.tables[key];
        if let SValue::Num(x) = &mut tbl.arr[si] {
            *x += 1.0;
            a2 += *x;
        }
    }
    let sm_ns = t.elapsed().as_nanos() as f64 / iters as f64;

    let mut plain: Vec<Vec<f64>> = (0..n_tables)
        .map(|t| (0..slots).map(|k| (t * slots + k) as f64).collect())
        .collect();
    let t = Instant::now();
    let mut rng = Rng(7);
    let mut a3 = 0.0;
    for _ in 0..iters {
        let ti = rng.idx(n_tables);
        let si = rng.idx(slots);
        plain[ti][si] += 1.0;
        a3 += plain[ti][si];
    }
    let plain_ns = t.elapsed().as_nanos() as f64 / iters as f64;

    black_box(a1);
    black_box(a2);
    black_box(a3);
    println!(
        "[throughput] iters={iters} (table[i][j]+=1, data-dep idx)\n  gc-arena unlock  = {gc_ns:.3} ns/op\n  slotmap index    = {sm_ns:.3} ns/op\n  plain Vec<Vec>   = {plain_ns:.3} ns/op"
    );
}

// ====================== (b) full-collection cost vs live heap ======================
fn bench_full_collect() {
    println!("[b/full-collect] finish_cycle() cost vs live heap size (per-collection determinism tax):");
    for &(n_tables, slots) in &[(64usize, 16usize), (256, 16), (1024, 16), (4096, 16), (4096, 64)] {
        let mut arena = build_gc_arena(n_tables, slots);
        let live_objs = n_tables + 2;
        let heap_bytes = arena.metrics().total_gc_allocation();
        arena.finish_cycle();
        arena.finish_cycle();
        let reps = 200;
        let t = Instant::now();
        for _ in 0..reps {
            arena.finish_cycle();
        }
        let per = t.elapsed().as_nanos() as f64 / reps as f64;
        let per_obj = per / live_objs as f64;
        println!(
            "  tables={n_tables:>4} slots={slots:>3}  liveObjs~{live_objs:>5}  heap={heap_bytes:>8}B  finish_cycle = {per:>10.1} ns  ({per_obj:.2} ns/liveObj)"
        );
    }
}

// ====================== (c) RETAIN pattern + serde OUT outside mutate ======================
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
struct Mirror {
    arr: Vec<f64>,
}

fn bench_retain_pattern() {
    let mut arena = build_gc_arena(8, 8);

    let t = Instant::now();
    let handle: DynamicRoot<Rootable![RefLock<GTable<'_>>]> = arena.mutate(|mc, root| {
        let mut arr = Vec::new();
        for i in 0..16 {
            arr.push(GValue::Num(i as f64));
        }
        let exported = Gc::new(mc, RefLock::new(GTable { arr }));
        root.roots.stash(mc, exported)
    });
    let stash_ns = t.elapsed().as_nanos();

    let ticks = 1000;
    let mut fetch_total = 0u128;
    let mut sum = 0.0;
    for tick in 0..ticks {
        arena.finish_cycle();
        arena.mutate(|mc, _root| {
            for j in 0..32 {
                let _g = Gc::new(mc, Static(format!("garbage-{tick}-{j}")));
            }
        });
        let tf = Instant::now();
        sum += arena.mutate(|mc, root| {
            let tbl = root.roots.fetch(&handle);
            let mut g = tbl.unlock(mc).borrow_mut();
            let n = g.arr.len();
            if let GValue::Num(x) = &mut g.arr[tick % n] {
                *x += 1.0;
                return *x;
            }
            0.0
        });
        fetch_total += tf.elapsed().as_nanos();
    }
    let fetch_ns = fetch_total as f64 / ticks as f64;

    let tdc = Instant::now();
    let owned: Mirror = arena.mutate(|_mc, root| {
        let tbl = root.roots.fetch(&handle);
        let g = tbl.borrow();
        let arr = g
            .arr
            .iter()
            .map(|v| match v {
                GValue::Num(n) => *n,
                _ => f64::NAN,
            })
            .collect();
        Mirror { arr }
    });
    let deepcopy_ns = tdc.elapsed().as_nanos();

    let tser = Instant::now();
    let json = serde_json::to_string(&owned).unwrap();
    let ser_ns = tser.elapsed().as_nanos();
    let back: Mirror = serde_json::from_str(&json).unwrap();
    assert_eq!(owned, back);

    black_box(sum);
    println!("[c/retain] survived {ticks} forced finish_cycle() collections; handle still valid: OK");
    println!("  stash (1x)                 = {stash_ns} ns");
    println!("  fetch+call per tick (avg)  = {fetch_ns:.1} ns  (the finish_cycle/tick is NOT in this number)");
    println!("  deep-copy OUT (16 f64)     = {deepcopy_ns} ns  -> owned Rust struct outside mutate");
    println!("  serde_json of owned mirror = {ser_ns} ns (json={} bytes)", json.len());

    println!("  --- stash/fetch vs deep-copy-out, by table size ---");
    for &len in &[4usize, 16, 64, 256, 1024] {
        let mut a = build_gc_arena(1, 1);
        let h: DynamicRoot<Rootable![RefLock<GTable<'_>>]> = a.mutate(|mc, root| {
            let arr = (0..len).map(|i| GValue::Num(i as f64)).collect();
            let exp = Gc::new(mc, RefLock::new(GTable { arr }));
            root.roots.stash(mc, exp)
        });
        let reps = 100_000;
        let t = Instant::now();
        let mut acc = 0.0;
        for _ in 0..reps {
            a.mutate(|_mc, root| {
                let tbl = root.roots.fetch(black_box(&h));
                acc += tbl.borrow().arr.len() as f64;
            });
        }
        let fetch = t.elapsed().as_nanos() as f64 / reps as f64;
        let t = Instant::now();
        for _ in 0..reps {
            let m: Mirror = a.mutate(|_mc, root| {
                let tbl = root.roots.fetch(&h);
                let g = tbl.borrow();
                Mirror {
                    arr: g
                        .arr
                        .iter()
                        .map(|v| match v {
                            GValue::Num(n) => *n,
                            _ => 0.0,
                        })
                        .collect(),
                }
            });
            acc += black_box(m).arr.len() as f64;
        }
        let deep = t.elapsed().as_nanos() as f64 / reps as f64;
        black_box(acc);
        println!("  len={len:>5}: fetch(root)={fetch:>8.2} ns   deep-copy-out={deep:>8.2} ns   ratio={:.1}x", deep / fetch);
    }
}

// ====================== (d) snapshot / rollback ======================
// The crux of gc-arena-vs-slotmap. Measure a full SNAPSHOT+RESTORE round-trip for each:
//  - slotmap: snapshot = clone(SlotMap)+clone(roots) [structural, preserves keys/identity/cycles];
//             restore = move the clone back (assign) -> dropping the current arena. O(heap) copy,
//             O(heap) drop, NO rebuild, identity preserved.
//  - gc-arena: NO snapshot primitive. snapshot = deep-flatten the live graph into an owned mirror;
//             restore = build a BRAND NEW arena from the mirror. Two arenas live at once; object
//             identity / aliasing / cycles are NOT preserved by a naive flatten.
fn bench_snapshot() {
    println!("[d/snapshot+restore] round-trip for the 24-deep rollback ring:");
    for &(n_tables, slots) in &[(256usize, 16usize), (1024, 16), (4096, 16)] {
        let reps = 500;

        // ---- slotmap: snapshot(clone) + restore(assign-back) ----
        let mut heap = build_slotmap_heap(n_tables, slots);
        let t = Instant::now();
        let mut keep = 0usize;
        for _ in 0..reps {
            // snapshot
            let snap_tables = heap.tables.clone();
            let snap_roots = heap.roots.clone();
            keep += snap_tables.len();
            // mutate something then restore
            if let Some(&k) = heap.roots.first() {
                if let SValue::Num(x) = &mut heap.tables[k].arr[0] {
                    *x += 1.0;
                }
            }
            // restore: drop current, swap snapshot back in
            heap.tables = black_box(snap_tables);
            heap.roots = black_box(snap_roots);
        }
        let sm_rt = t.elapsed().as_nanos() as f64 / reps as f64;

        // ---- gc-arena: snapshot(deep-flatten) + restore(rebuild fresh arena) ----
        let arena = build_gc_arena(n_tables, slots);
        let t = Instant::now();
        let mut acc = 0usize;
        for _ in 0..reps {
            // snapshot: flatten whole live graph to owned
            let mirror: Vec<Vec<f64>> = arena.mutate(|_mc, root| {
                root.tables
                    .borrow()
                    .iter()
                    .map(|tb| {
                        tb.borrow()
                            .arr
                            .iter()
                            .map(|v| match v {
                                GValue::Num(n) => *n,
                                _ => 0.0,
                            })
                            .collect()
                    })
                    .collect()
            });
            // restore: build a brand-new arena from the mirror (the only way back)
            let restored = GcArena::new(|mc| {
                let v: Vec<_> = mirror
                    .iter()
                    .map(|row| {
                        let arr = row.iter().map(|n| GValue::Num(*n)).collect();
                        Gc::new(mc, RefLock::new(GTable { arr }))
                    })
                    .collect();
                GcRoot {
                    tables: Gc::new(mc, RefLock::new(v)),
                    roots: DynamicRootSet::new(mc),
                }
            });
            acc += black_box(&restored).metrics().total_gc_allocation() as usize & 1;
        }
        let gc_rt = t.elapsed().as_nanos() as f64 / reps as f64;
        black_box(keep);
        black_box(acc);
        println!(
            "  tables={n_tables:>4} slots={slots:>3}: slotmap snap+restore = {sm_rt:>9.0} ns   gc-arena flatten+rebuild = {gc_rt:>9.0} ns   ({:.1}x)  [gc loses identity/cycles]",
            gc_rt / sm_rt
        );
    }
}

// ====================== Send move path ======================
struct SendArena(GcArena);
// SAFETY: whole arena moves as a unique owner; no arena-derived handle (Gc / Metrics /
// DynamicRoot's Rc) is shared across the boundary.
unsafe impl Send for SendArena {}

fn test_send_move() {
    let arena = SendArena(build_gc_arena(16, 8));
    let h = std::thread::spawn(move || {
        let a = arena;
        a.0.mutate(|mc, root| {
            let mut g = root.tables.borrow()[0].unlock(mc).borrow_mut();
            if let GValue::Num(x) = &mut g.arr[0] {
                *x += 100.0;
            }
            root.tables.borrow().len()
        })
    });
    println!("[send] moved arena to pool thread; tables={}", h.join().unwrap());
}

fn main() {
    println!("=== Spike D: gc-arena vs slotmap ===");
    bench_barrier_isolated(20_000_000);
    bench_barrier_marking(2_000_000);
    bench_throughput(50_000_000);
    bench_full_collect();
    bench_retain_pattern();
    bench_snapshot();
    test_send_move();
    println!("=== done ===");
}
