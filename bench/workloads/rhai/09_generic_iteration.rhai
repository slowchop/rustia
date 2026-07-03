// Rhai iteration workload: iterate arrays and a small object map.
fn f(n) {
    let items = [];
    let i = 1;
    while i <= 32 {
        items.push(i * i);
        i += 1;
    }

    let hashmap = #{ a: 1, b: 2, c: 3, d: 4, e: 5 };
    let acc = 0;
    let it = 0;
    while it < n {
        for v in items {
            acc += v;
        }
        for v in hashmap.values() {
            acc += v;
        }
        for v in items {
            acc += v;
        }
        it += 1;
    }
    acc
}
