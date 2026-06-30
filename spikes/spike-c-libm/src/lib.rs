// Spike C: f64 / pure-Rust libm bit-identity probe.
// std is fine; compute() itself only touches core f64 ops + libm, so the exact
// same code path compiles for wasm32-unknown-unknown.

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[inline]
fn fnv1a_u64(mut h: u64, bits: u64) -> u64 {
    let bytes = bits.to_le_bytes();
    let mut i = 0;
    while i < 8 {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    h
}

// Canonicalize: collapse all NaN payloads to one bit pattern and -0.0 -> +0.0.
#[inline]
fn canon_bits(v: f64) -> u64 {
    if v.is_nan() {
        0x7ff8000000000000 // canonical quiet NaN
    } else if v == 0.0 {
        0 // maps both +0.0 and -0.0 to +0.0 bits
    } else {
        v.to_bits()
    }
}

#[inline]
fn raw_bits(v: f64) -> u64 {
    v.to_bits()
}

// Build the deterministic input sweep. Returns count; fills `out` up to its len.
fn inputs(out: &mut [f64]) -> usize {
    let mut n = 0;
    let mut push = |buf: &mut [f64], idx: &mut usize, x: f64| {
        if *idx < buf.len() {
            buf[*idx] = x;
            *idx += 1;
        }
    };

    // Determinism-hostile / special values.
    let specials: [f64; 24] = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        2.0,
        -2.0,
        0.5,
        -0.5,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        f64::MIN,
        f64::MAX,
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
        f64::from_bits(1),                  // smallest subnormal
        f64::from_bits(0x000fffffffffffff), // largest subnormal
        f64::from_bits(0x8000000000000001), // negative subnormal
        1e300,
        -1e300,
        1e-300,
        -1e-300,
        core::f64::consts::PI,
        core::f64::consts::E,
    ];
    for &s in specials.iter() {
        push(out, &mut n, s);
    }

    // Dense linear sweep over [-12, 12].
    let steps = 600usize;
    let mut i = 0;
    while i < steps {
        let t = (i as f64) / (steps as f64); // 0..1
        let x = -12.0 + t * 24.0;
        push(out, &mut n, x);
        i += 1;
    }

    // Log-magnitude sweep over many decades, both signs.
    let mut e = -300i32;
    while e <= 300 {
        let mag = libm::pow(10.0, e as f64);
        push(out, &mut n, mag);
        push(out, &mut n, -mag);
        // a couple of off-grid mantissas
        push(out, &mut n, mag * 1.2345678901234567);
        push(out, &mut n, -mag * 7.7654321098765432);
        e += 1;
    }

    // Fine sweep near 0 and near transcendental danger zones.
    let mut k = 0;
    while k < 400 {
        let x = (k as f64) * 0.001 - 0.2; // -0.2 .. 0.2
        push(out, &mut n, x);
        k += 1;
    }

    n
}

#[derive(Clone, Copy)]
pub struct FnHash {
    pub canon: u64,
    pub raw: u64,
}

// Order MUST stay stable; node/wasm harness indexes by this order.
pub const FN_NAMES: [&str; 10] = [
    "sin", "cos", "exp", "ln", "pow", "sqrt", "fmod", "tan", "atan2", "coreops",
];

pub struct Report {
    pub per_fn: [FnHash; 10],
    pub combined_canon: u64,
    pub combined_raw: u64,
    // Raw sample bit patterns for the determinism-hostile probes.
    pub samples: [(u64, u64); 12], // (canon_bits, raw_bits)
}

pub fn compute() -> Report {
    let mut buf = [0.0f64; 4096];
    let n = inputs(&mut buf);
    let xs = &buf[..n];

    let mut per = [FnHash { canon: FNV_OFFSET, raw: FNV_OFFSET }; 10];

    // Unary functions.
    for &x in xs {
        let r = libm::sin(x);
        per[0].canon = fnv1a_u64(per[0].canon, canon_bits(r));
        per[0].raw = fnv1a_u64(per[0].raw, raw_bits(r));

        let r = libm::cos(x);
        per[1].canon = fnv1a_u64(per[1].canon, canon_bits(r));
        per[1].raw = fnv1a_u64(per[1].raw, raw_bits(r));

        let r = libm::exp(x);
        per[2].canon = fnv1a_u64(per[2].canon, canon_bits(r));
        per[2].raw = fnv1a_u64(per[2].raw, raw_bits(r));

        let r = libm::log(x);
        per[3].canon = fnv1a_u64(per[3].canon, canon_bits(r));
        per[3].raw = fnv1a_u64(per[3].raw, raw_bits(r));

        let r = libm::sqrt(x);
        per[5].canon = fnv1a_u64(per[5].canon, canon_bits(r));
        per[5].raw = fnv1a_u64(per[5].raw, raw_bits(r));

        let r = libm::tan(x);
        per[7].canon = fnv1a_u64(per[7].canon, canon_bits(r));
        per[7].raw = fnv1a_u64(per[7].raw, raw_bits(r));
    }

    // Binary functions: cross product over a stride-limited subset for size.
    let m = xs.len();
    let stride = if m > 64 { m / 64 } else { 1 };
    let mut ai = 0;
    while ai < m {
        let a = xs[ai];
        let mut bi = 0;
        while bi < m {
            let b = xs[bi];

            let r = libm::pow(a, b);
            per[4].canon = fnv1a_u64(per[4].canon, canon_bits(r));
            per[4].raw = fnv1a_u64(per[4].raw, raw_bits(r));

            let r = libm::fmod(a, b);
            per[6].canon = fnv1a_u64(per[6].canon, canon_bits(r));
            per[6].raw = fnv1a_u64(per[6].raw, raw_bits(r));

            let r = libm::atan2(a, b);
            per[8].canon = fnv1a_u64(per[8].canon, canon_bits(r));
            per[8].raw = fnv1a_u64(per[8].raw, raw_bits(r));

            bi += stride;
        }
        ai += stride;
    }

    // Core ops: +,-,*,/,sqrt,min,max and explicit NaN-propagation of f64::min/max.
    {
        let acc = &mut per[9];
        for &a in xs {
            for &b in [1.0, -1.0, 0.0, -0.0, f64::NAN, f64::INFINITY, 3.0, 0.1].iter() {
                let ops = [
                    a + b,
                    a - b,
                    a * b,
                    a / b,
                    libm::sqrt(a * b),
                    a.min(b), // f64::min — NaN propagation behavior under test
                    a.max(b), // f64::max
                ];
                for &r in ops.iter() {
                    acc.canon = fnv1a_u64(acc.canon, canon_bits(r));
                    acc.raw = fnv1a_u64(acc.raw, raw_bits(r));
                }
            }
        }
        // Explicit NaN-propagation probes (folded raw, no canonicalization on these
        // two extra rows so divergence in min/max NaN handling would show in raw).
        let nan = f64::NAN;
        let probes = [
            nan.min(1.0),
            (1.0f64).min(nan),
            nan.max(1.0),
            (1.0f64).max(nan),
            nan.min(nan),
            nan.max(nan),
        ];
        for &r in probes.iter() {
            acc.canon = fnv1a_u64(acc.canon, canon_bits(r));
            acc.raw = fnv1a_u64(acc.raw, raw_bits(r));
        }
    }

    // Combined hashes fold all per-fn hashes in order.
    let mut cc = FNV_OFFSET;
    let mut cr = FNV_OFFSET;
    for h in per.iter() {
        cc = fnv1a_u64(cc, h.canon);
        cr = fnv1a_u64(cr, h.raw);
    }

    // Determinism-hostile sample bit patterns.
    let samples = [
        (canon_bits(0.0f64 / 0.0f64), raw_bits(0.0f64 / 0.0f64)), // 0/0 NaN
        (canon_bits(libm::sqrt(-1.0)), raw_bits(libm::sqrt(-1.0))), // sqrt(-1)
        (canon_bits(libm::log(-1.0)), raw_bits(libm::log(-1.0))),   // ln(-1)
        (canon_bits(-0.0f64), raw_bits(-0.0f64)),                   // -0.0
        (canon_bits(1.0 / 0.0), raw_bits(1.0 / 0.0)),               // +inf
        (canon_bits(-1.0 / 0.0), raw_bits(-1.0 / 0.0)),             // -inf
        (canon_bits(f64::INFINITY - f64::INFINITY), raw_bits(f64::INFINITY - f64::INFINITY)), // inf-inf NaN
        (canon_bits(0.0 * f64::INFINITY), raw_bits(0.0 * f64::INFINITY)), // 0*inf NaN
        (canon_bits(libm::pow(0.0, 0.0)), raw_bits(libm::pow(0.0, 0.0))), // pow(0,0)
        (canon_bits(libm::pow(-1.0, 0.5)), raw_bits(libm::pow(-1.0, 0.5))), // pow(-1,0.5) NaN
        (canon_bits(f64::NAN.min(1.0)), raw_bits(f64::NAN.min(1.0))), // f64::min(NaN,1)
        (canon_bits(f64::NAN.max(1.0)), raw_bits(f64::NAN.max(1.0))), // f64::max(NaN,1)
    ];

    Report { per_fn: per, combined_canon: cc, combined_raw: cr, samples }
}

// ---- wasm32-unknown-unknown exports (no_std, no allocator needed) ----

// Order of flattened values for the wasm harness:
//   for i in 0..10: per_fn[i].canon, per_fn[i].raw   (20 values)
//   combined_canon, combined_raw                      (2 values)
//   for s in 0..12: samples[s].canon, samples[s].raw  (24 values)
// total 46.
#[no_mangle]
pub extern "C" fn wasm_n() -> u32 {
    46
}

#[no_mangle]
pub extern "C" fn wasm_hash(i: u32) -> u64 {
    let r = compute();
    let i = i as usize;
    if i < 20 {
        let f = &r.per_fn[i / 2];
        if i % 2 == 0 { f.canon } else { f.raw }
    } else if i == 20 {
        r.combined_canon
    } else if i == 21 {
        r.combined_raw
    } else {
        let s = i - 22;
        let pair = &r.samples[s / 2];
        if s % 2 == 0 { pair.0 } else { pair.1 }
    }
}
