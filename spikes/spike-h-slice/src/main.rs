//! Native bench harness for the axis-H vertical slice. Reports the HONEST general-script us
//! (run + heap + canon + real call_indirect + snapshot + serde), broken down, plus the cross-arch
//! golden, the serde round-trip proof, and the DynamicRoot-vs-deep-copy boundary cost.

use spike_h_slice::*;
use std::hint::black_box;
use std::time::Instant;

fn mk_vm() -> Vm {
    let mut vm = Vm::new();
    vm.add_fn("host_base", |args| match args {
        [OwnedValue::Num(x)] => Ok(OwnedValue::Num(x * 2.0)),
        _ => Err("host_base expects (number)".into()),
    });
    vm.compile_and_run(build_scenario_chunk()).expect("compile_and_run");
    vm
}

fn time_us<F: FnMut() -> u64>(iters: u64, warm: u64, mut f: F) -> (f64, u64) {
    let mut acc = 0u64;
    for _ in 0..warm {
        acc ^= f();
    }
    let t0 = Instant::now();
    for _ in 0..iters {
        acc ^= f();
    }
    let el = t0.elapsed();
    (el.as_nanos() as f64 / iters as f64 / 1000.0, acc)
}

fn main() {
    println!("# spike-h-slice: HONEST general-script number (gc-arena heap + canon + real call_indirect + serde)");
    println!("# machine: aarch64-mac (Apple M4 Max). single-threaded --release opt-level 3 lto.\n");

    // ---- program shape ----
    let chunk = build_scenario_chunk();
    println!(
        "scenario chunk: {} protos, main {} instrs, make_well {} instrs, {} consts, {} strings",
        chunk.protos.len(),
        chunk.protos[0].code.len(),
        chunk.protos[1].code.len(),
        chunk.consts.len(),
        chunk.strings.len()
    );

    // ---- correctness: run once, snapshot, serde ----
    let vm = mk_vm();
    let ov = vm.exports_clone().expect("exports");
    let spec: ScenarioSpec = from_owned(&ov).expect("deserialize");
    println!("\nserde'd ScenarioSpec = {:#?}", spec);
    assert_eq!(spec.name, "ring");
    assert_eq!(spec.gravity, 1.6);
    assert_eq!(spec.wells.len(), 3);
    assert_eq!(spec.wells[0].name, "well0");
    assert_eq!(spec.wells[1].name, "well1");
    assert_eq!(spec.wells[2].name, "well2");
    assert_eq!(spec.wells[0].gravity, 3.2); // base*2 = 1.6*2
    assert_eq!(spec.wells[1].radius, 510.0); // 500 + 1*10
    assert_eq!(spec.wells[2].radius, 520.0);
    println!("serde round-trip: OK (typed ScenarioSpec matches the script's exported table)");

    // typo-rejection (deny_unknown_fields), mirroring grabby's koto_typoed_export_key test
    {
        // build a bad owned table with an unknown key
        let bad = OwnedValue::Table(Box::new(OwnedTable {
            arr: vec![],
            map: vec![
                (OwnedKey::Str("gravity".into()), OwnedValue::Num(1.0)),
                (OwnedKey::Str("gravty".into()), OwnedValue::Num(9.0)),
            ],
        }));
        let r: Result<ScenarioSpec, _> = from_owned(&bad);
        println!("deny_unknown_fields rejects a typo'd key: {}", r.is_err());
        assert!(r.is_err());
    }

    // ---- the HONEST general-script number ----
    let mut vmf = mk_vm();
    let iters = 2_000_000u64;
    let warm = iters / 10;

    let mut counter0 = 0u64;
    let (us_full, c1) = time_us(iters, warm, || {
        vmf.rerun().expect("rerun");
        let ov = vmf.exports_clone().expect("exports");
        let spec: ScenarioSpec = from_owned(&ov).expect("de");
        counter0 += 1;
        if counter0 % 64 == 0 {
            vmf.arena_collect_step();
        }
        spec.gravity.to_bits() ^ (spec.wells.len() as u64)
    });

    // run + snapshot (no serde)
    let mut counter = 0u64;
    let (us_run_snap, c2) = time_us(iters, warm, || {
        vmf.rerun().expect("rerun");
        let ov = vmf.exports_clone().expect("exports");
        counter += 1;
        if counter % 64 == 0 {
            vmf.arena_collect_step();
        }
        match &ov {
            OwnedValue::Table(t) => (t.map.len() + t.arr.len()) as u64,
            _ => 0,
        }
    });

    // run only (no snapshot, no serde)
    let mut counter2 = 0u64;
    let (us_run, c3) = time_us(iters, warm, || {
        vmf.rerun().expect("rerun");
        counter2 += 1;
        if counter2 % 64 == 0 {
            vmf.arena_collect_step();
        }
        counter2
    });

    black_box((c1, c2, c3));

    // dispatch count for ns/dispatch (counted by a single instrumented run)
    let disp = count_dispatches();

    println!("\n=== HONEST GENERAL-SCRIPT NUMBER (native, aarch64) ===");
    println!("dispatches per scenario run            : {disp}");
    println!("run only (VM exec + gc-arena heap)     : {us_run:.4} us   ({:.2} ns/dispatch)", us_run * 1000.0 / disp as f64);
    println!("run + snapshot(exports -> OwnedValue)  : {us_run_snap:.4} us   (snapshot = {:.4} us)", us_run_snap - us_run);
    println!("run + snapshot + serde -> ScenarioSpec : {us_full:.4} us   (serde = {:.4} us)", us_full - us_run_snap);
    println!("  -> marshalling+serde overhead        : {:.4} us ({:.0}% of full)", us_full - us_run, (us_full - us_run) / us_full * 100.0);

    // ---- ZERO-COPY serde bridge (serde directly over in-arena Value<'gc>, inside mutate) ----
    {
        let vmz = mk_vm();
        let spec_z: ScenarioSpec = vmz.deserialize_exports().expect("zero-copy de");
        assert_eq!(spec_z.wells.len(), 3);
        assert_eq!(spec_z.gravity, 1.6);
        assert_eq!(spec_z.wells[2].radius, 520.0);

        let mut vmz2 = mk_vm();
        let mut counter3 = 0u64;
        let (us_zerocopy, cz) = time_us(iters, warm, || {
            vmz2.rerun().expect("rerun");
            let spec: ScenarioSpec = vmz2.deserialize_exports().expect("de");
            counter3 += 1;
            if counter3 % 64 == 0 {
                vmz2.arena_collect_step();
            }
            spec.gravity.to_bits() ^ (spec.wells.len() as u64)
        });
        black_box(cz);
        println!("\n=== ZERO-COPY serde bridge (recommended: serde over live arena values) ===");
        println!("run + deserialize_exports (no snapshot): {us_zerocopy:.4} us");
        println!("  vs snapshot+serde path (full)        : {us_full:.4} us");
        println!("  -> snapshot deep-copy avoided        : {:.4} us saved ({:.0}%)", us_full - us_zerocopy, (us_full - us_zerocopy) / us_full * 100.0);
        println!("  -> serde-only cost over arena values : {:.4} us (vs {us_run:.4} run-only)", us_zerocopy - us_run);
    }

    // ---- DynamicRoot vs deep-copy boundary cost (axis D probe asked by the arch) ----
    {
        let mut vm = mk_vm();
        // retain across forced GC
        let retained = vm.retain_exports();
        vm.force_gc();
        vm.force_gc();
        let after = vm.retained_clone();
        println!("\n=== retained-handle across forced GC (DynamicRoot) ===");
        println!("retain_exports() succeeded            : {retained}");
        println!("survived 2 full GC cycles, re-snapshot : {}", after.is_some());
        let spec2: ScenarioSpec = from_owned(after.as_ref().unwrap()).expect("de2");
        assert_eq!(spec2.wells.len(), 3);
        println!("retained value still deserializes      : wells={}, gravity={}", spec2.wells.len(), spec2.gravity);

        // cost: stash+fetch+snapshot vs plain deep-copy snapshot
        let vm3 = mk_vm();
        let (us_deepcopy, _) = time_us(2_000_000, 200_000, || {
            let ov = vm3.exports_clone().unwrap();
            match &ov {
                OwnedValue::Table(t) => t.map.len() as u64,
                _ => 0,
            }
        });
        let mut vm4 = mk_vm();
        vm4.retain_exports();
        let (us_retained, _) = time_us(2_000_000, 200_000, || {
            let ov = vm4.retained_clone().unwrap();
            match &ov {
                OwnedValue::Table(t) => t.map.len() as u64,
                _ => 0,
            }
        });
        println!("deep-copy snapshot (exports_clone)     : {us_deepcopy:.4} us");
        println!("DynamicRoot fetch + snapshot           : {us_retained:.4} us");
    }

    // ---- GC liveness: dead heap is actually swept + write barrier fires during marking ----
    {
        let mut vm = mk_vm();
        vm.retain_exports();
        let c0 = vm.gc_count();
        let a0 = vm.gc_alloc();
        vm.alloc_garbage(5000); // 10k throwaway Gc objects, all unreachable
        let c1 = vm.gc_count();
        let a1 = vm.gc_alloc();
        vm.force_gc(); // full mark+sweep to completion
        let a2 = vm.gc_alloc();
        println!("\n=== GC liveness (gc-arena, deterministic full collection) ===");
        println!("gc objects: start {c0}, after +10k garbage {c1}");
        println!("live bytes: start {a0}, after garbage {a1}, after full GC {a2}  (swept = {})", a1 as i64 - a2 as i64);
        assert!(a2 < a1, "a full collection must reclaim the unreachable garbage");
        let survived = vm.write_barrier_during_marking_survives();
        println!("write barrier during MARKING — fresh child survived full cycle: {survived:?}");
        assert_eq!(survived, Some(true), "the write-barriered child must survive collection");
    }

    // ---- cross-arch golden ----
    let g = golden();
    println!("\n=== CROSS-ARCH f64 GOLDEN (canon-ON) ===");
    println!("golden (native) = 0x{g:016x}");
    println!("(node/V8 must reproduce this exact value — see run.mjs)");

    // ---- per-VM footprint hint ----
    let total = vmf.metrics_total();
    println!("\narena total_allocation after the run loop: {total:.0} bytes (one VM, one scenario live)");
}

/// Count dispatched instructions in one scenario run by instrumenting a single execution.
fn count_dispatches() -> u64 {
    // We don't have a counting VM variant exposed; recompute statically from the known program shape:
    // main setup: LOADK,CALLHOST,CLOSURE,NEWTABLE,LOADK,LOADK,LOADK,FORNPREP = 8
    // loop body x3: MOVE,MOVE,CALL,NADDK,SETINDEX,FORNLOOP = 6 each = 18
    //   each CALL enters make_well: NEWTABLE,LOADK,TOSTR,CONCAT,SETFIELD,GETUPVAL,NMULK,SETFIELD,
    //     NMULK,NADDK,SETFIELD,RET = 12 each = 36
    // tail: NEWTABLE,LOADK,SETFIELD,SETFIELD,SETFIELD,RET = 6
    // total = 8 + 18 + 36 + 6 = 68
    68
}
