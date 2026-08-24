// Rhai in-place array mutation: repeated read-modify-write over a persistent array.
fn f(n) {
    let t = [];
    let i = 1;
    while i <= 256 {
        t.push(i);
        i += 1;
    }

    let checksum = 0;
    let it = 1;
    while it <= n {
        let j = 0;
        while j < 256 {
            t[j] = (t[j] * 3 + it) % 1000;
            j += 1;
        }
        checksum += t[0] + t[255];
        it += 1;
    }
    checksum
}
