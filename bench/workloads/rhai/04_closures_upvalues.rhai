// Rhai closure/upvalue-shaped workload: mutate eight counters through a small dispatch function.
fn next_counter(counters, idx) {
    counters[idx] += idx + 1;
    counters[idx]
}

fn f(n) {
    let acc = 0;
    let counters = [1, 2, 3, 4, 5, 6, 7, 8];
    let i = 0;
    while i < n {
        let idx = i % 8;
        acc += next_counter(counters, idx);
        i += 1;
    }
    acc
}
