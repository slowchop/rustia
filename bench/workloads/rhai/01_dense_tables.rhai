// Rhai dense array workload: build a 256-element array, sum it, sort it, read endpoints.
fn f(n) {
    let checksum = 0;
    let it = 0;
    while it < n {
        let t = [];
        let i = 1;
        while i <= 256 {
            t.push((i * 2654435761 + it) % 1000);
            i += 1;
        }

        let s = 0;
        let j = 0;
        while j < 256 {
            s += t[j];
            j += 1;
        }

        t.sort();
        checksum += t[0] + t[255] + s;
        it += 1;
    }
    checksum
}
