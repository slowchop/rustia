// Rhai math/array workload: math helpers plus buffered push/sort/flush.
fn f(n) {
    let acc = 0.0;
    let t = [];
    let i = 1;
    while i <= n {
        let x = i.to_float() * 0.001;
        acc += sin(x) * cos(x) + sqrt(abs(x));
        t.push(floor(acc).to_int() % 997);
        if t.len() >= 64 {
            t.sort();
            acc += t[0].to_float();
            t = [];
        }
        i += 1;
    }
    acc
}
