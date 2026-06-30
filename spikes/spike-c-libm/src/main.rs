use spike_c_libm::{compute, FN_NAMES};

fn main() {
    let r = compute();
    println!("=== spike-c libm bit-identity report ===");
    println!("target_arch={} pointer_width={}", std::env::consts::ARCH, usize::BITS);
    println!("--- per-function hashes (canon / raw) ---");
    for (i, name) in FN_NAMES.iter().enumerate() {
        println!(
            "{:>7}: canon=0x{:016x} raw=0x{:016x}",
            name, r.per_fn[i].canon, r.per_fn[i].raw
        );
    }
    println!("--- combined ---");
    println!("combined_canon=0x{:016x}", r.combined_canon);
    println!("combined_raw  =0x{:016x}", r.combined_raw);
    println!("--- determinism-hostile sample bit patterns (canon_bits / raw_bits) ---");
    let labels = [
        "0.0/0.0",
        "sqrt(-1)",
        "ln(-1)",
        "-0.0",
        "+inf",
        "-inf",
        "inf-inf",
        "0*inf",
        "pow(0,0)",
        "pow(-1,0.5)",
        "f64::min(NaN,1)",
        "f64::max(NaN,1)",
    ];
    for (i, l) in labels.iter().enumerate() {
        println!(
            "{:>16}: canon=0x{:016x} raw=0x{:016x}",
            l, r.samples[i].0, r.samples[i].1
        );
    }
}
