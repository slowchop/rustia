use rustia::RustiaBuilder;

#[test]
fn puc51_compat_preset_enables_loadstring_and_puc_semantics() {
    let mut rustia = RustiaBuilder::puc51_compat().build().unwrap();

    let answer: f64 = rustia.run("return loadstring('return 40 + 2')()").unwrap();
    assert_eq!(answer, 42.0);

    // PUC %.14g number formatting.
    let text: String = rustia.run("return tostring(1e15)").unwrap();
    assert_eq!(text, "1e+15");

    // PUC table layout: hash-part iteration order, not insertion order
    // (matches the offline 5.1.5 oracle for this exact build sequence).
    let order: String = rustia
        .run(
            "local t = {} \
             t.alpha = 1 t.beta = 2 t.gamma = 3 t.delta = 4 t.eps = 5 \
             local out = {} \
             for k in pairs(t) do out[#out + 1] = k end \
             return table.concat(out, ',')",
        )
        .unwrap();
    assert_eq!(order, "beta,eps,delta,gamma,alpha");
}
