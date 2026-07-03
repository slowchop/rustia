// Rhai sparse map workload: fill string and scattered numeric-shaped keys, then read them back.
fn f(n) {
    let checksum = 0;
    let it = 0;
    while it < n {
        let t = #{};
        let i = 1;
        while i <= 64 {
            t["k" + i] = i * it;
            t["n" + ((i * 7919) % 100003)] = i;
            i += 1;
        }

        let s = 0;
        let j = 1;
        while j <= 64 {
            s += t["k" + j] + t["n" + ((j * 7919) % 100003)];
            j += 1;
        }

        checksum += s;
        it += 1;
    }
    checksum
}
