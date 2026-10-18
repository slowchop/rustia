use rustia::{ErrorKind, Profile, RustiaBuilder};

pub const NUMERIC_APPEND: &str = "local t = {} for i = 1, 5000000 do t[i] = i end return #t";
pub const STRING_APPEND: &str =
    "local t = {} local s = string.rep('x', 32) for i = 1, 5000000 do t[i] = s end return #t";

pub fn headroom_limit(profile: Profile, extra: usize) -> usize {
    let mut baseline = RustiaBuilder::standard().profile(profile).build().unwrap();
    baseline.memory_stats().live_bytes().saturating_add(extra)
}

fn assert_out_of_memory(result: &Result<i64, rustia::Error>) {
    assert!(
        matches!(result, Err(e) if matches!(e.kind(), ErrorKind::Vm(v)
            if matches!(v.kind(), rustia::VmErrorKind::OutOfMemory))),
        "expected OutOfMemory, got {result:?}"
    );
}

pub fn assert_append_matches_scalar(
    source: &str,
    cadence: &str,
    mut make_vm: impl FnMut(Profile, Option<u64>, usize) -> rustia::Rustia,
) {
    for profile in [
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::LuauCompat,
        Profile::Puc51Compat,
    ] {
        let limit = headroom_limit(profile, 8 * 1024);
        let mut vm = make_vm(profile, None, limit);
        assert_out_of_memory(&vm.eval::<i64>(source));
        let unmetered = vm.memory_stats();
        // A fuel budget independently forces scalar dispatch. It is large
        // enough that memory, rather than fuel, is the failing resource.
        let mut vm = make_vm(profile, Some(100_000_000), limit);
        assert_out_of_memory(&vm.eval::<i64>(source));
        let scalar = vm.memory_stats();
        assert_eq!(
            unmetered.live_bytes(),
            scalar.live_bytes(),
            "quota failure must leave the same live memory: {profile:?}, cadence {cadence}"
        );
        assert_eq!(
            unmetered.peak_live_bytes(),
            scalar.peak_live_bytes(),
            "no unchecked allocation window: {profile:?}, cadence {cadence}"
        );
        let peak = unmetered.peak_live_bytes().expect("tracking is enabled");
        assert!(
            peak.saturating_sub(limit) < 200,
            "quota enforcement must retain single-instruction precision: \
                 {profile:?}, cadence {cadence}, peak {peak}, limit {limit}"
        );
    }
}
