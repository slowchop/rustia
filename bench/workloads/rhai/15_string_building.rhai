// Rhai string-building workload: build 64 fragments through repeated concatenation.
fn f(n) {
    let fragments = ["alpha", "beta", "gamma", "delta"];
    let checksum = 0;
    let it = 0;
    while it < n {
        let s = "";
        let i = 0;
        while i < 64 {
            let fragment = fragments[(i + it) % fragments.len()];
            s += fragment + ":" + (i % 17).to_string() + ":" + (it % 31).to_string() + "|";
            i += 1;
        }
        checksum += s.len();
        it += 1;
    }
    checksum
}
