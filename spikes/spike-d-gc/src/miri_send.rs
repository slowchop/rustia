//! miri target for the `unsafe impl Send` move path (design §2-D gate).
//! Run with: cargo +nightly miri run --bin miri_send
//! Tiny iteration counts (miri is ~100-1000x slower than native).
//!
//! What we are checking for UB:
//!  1) moving the whole Arena to another thread and mutating there (the N-VM pool model),
//!  2) the DynamicRoot handle (!Send: holds Weak<RefCell>) moving *with* the arena as one unit
//!     and being fetched on the destination thread,
//!  3) a forced full GC cycle on the destination thread (touches every GcBox via raw NonNull),
//!  4) moving the arena BACK to a third thread (simulating return-to-pool).

use gc_arena::barrier::Unlock;
use gc_arena::lock::RefLock;
use gc_arena::{Arena, Collect, DynamicRoot, DynamicRootSet, Gc, Rootable, Static};

#[derive(Collect, Clone)]
#[collect(no_drop)]
enum GValue<'gc> {
    #[allow(dead_code)]
    Nil,
    Num(f64),
    Str(Gc<'gc, Static<String>>),
}

#[derive(Collect)]
#[collect(no_drop)]
struct GTable<'gc> {
    arr: Vec<GValue<'gc>>,
}

#[derive(Collect)]
#[collect(no_drop)]
struct Root<'gc> {
    t: Gc<'gc, RefLock<GTable<'gc>>>,
    roots: DynamicRootSet<'gc>,
}
type MyArena = Arena<Rootable![Root<'_>]>;

// The Send unit: the arena AND any retained !Send handles travel together.
struct VmCell {
    arena: MyArena,
    exported: DynamicRoot<Rootable![RefLock<GTable<'_>>]>,
}
// SAFETY: VmCell is the sole owner of the arena and of every handle derived from it. Moving the
// cell moves the whole graph + its DynamicRootSet's Rc + the DynamicRoot's Weak as one unit; none
// is aliased on the source thread afterwards (move semantics), so no Gc NonNull and no non-atomic
// Rc/Weak refcount is ever touched from two threads concurrently.
unsafe impl Send for VmCell {}

fn build() -> VmCell {
    let mut arena = MyArena::new(|mc| Root {
        t: Gc::new(mc, RefLock::new(GTable { arr: vec![GValue::Num(1.0)] })),
        roots: DynamicRootSet::new(mc),
    });
    let exported = arena.mutate(|mc, root| {
        let arr = vec![GValue::Num(10.0), GValue::Str(Gc::new(mc, Static("hi".into())))];
        let exp = Gc::new(mc, RefLock::new(GTable { arr }));
        root.roots.stash(mc, exp)
    });
    VmCell { arena, exported }
}

fn tick(cell: &mut VmCell) -> f64 {
    cell.arena.mutate(|mc, _root| {
        for j in 0..3 {
            let _g = Gc::new(mc, Static(format!("g{j}")));
        }
    });
    cell.arena.finish_cycle();
    let exported = &cell.exported;
    cell.arena.mutate(|mc, root| {
        let tbl = root.roots.fetch(exported);
        let mut g = tbl.unlock(mc).borrow_mut();
        if let GValue::Num(x) = &mut g.arr[0] {
            *x += 1.0;
            return *x;
        }
        0.0
    })
}

fn main() {
    let mut cell = build();
    let _ = tick(&mut cell);

    let cell = std::thread::spawn(move || {
        let mut cell = cell;
        let mut last = 0.0;
        for _ in 0..3 {
            last = tick(&mut cell);
        }
        println!("[miri] thread1 last={last}");
        cell
    })
    .join()
    .unwrap();

    let n = std::thread::spawn(move || {
        let mut cell = cell;
        let v = tick(&mut cell);
        let len = cell.arena.mutate(|_mc, root| root.t.borrow().arr.len());
        println!("[miri] thread2 v={v} live_len={len}");
        v
    })
    .join()
    .unwrap();

    println!("[miri] OK final={n}");
}
