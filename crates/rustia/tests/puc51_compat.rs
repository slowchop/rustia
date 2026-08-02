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

    // PUC constructor write order: the trailing positional flush overwrites an
    // earlier keyed write (reference Luau keeps source order instead).
    let slot: f64 = rustia
        .run("local t = {1, 2, 3, [1] = 9, 4} return t[1]")
        .unwrap();
    assert_eq!(slot, 1.0);

    // Runtime-compiled chunks (loadstring) follow the same PUC write order.
    let loaded_slot: f64 = rustia
        .run("return loadstring('local t = {1, [1] = 9} return t[1]')()")
        .unwrap();
    assert_eq!(loaded_slot, 1.0);
}

#[test]
fn puc51_compat_table_create_builtin_is_not_a_constructor_hint() {
    let mut rustia = RustiaBuilder::puc51_compat().build().unwrap();

    // One-arg table.create(n) must not be mistaken for a compiler constructor
    // hint: the aux marker space is disjoint (constructor aux == 0, builtin
    // arity 1..=2). Register 0 holding a non-number previously produced a
    // spurious "type error: expected number"; a large number silently sized
    // the node part from unrelated data.
    let size: f64 = rustia
        .run(
            "local s = 'str' \
             local t = table.create(5) \
             local u = table.create(3, 'v') \
             return #t + #u + (u[3] == 'v' and 100 or 0)",
        )
        .unwrap();
    assert_eq!(size, 103.0);

    // The builtin's size cap still applies (constructor hints bypass it, but
    // only for actual constructors).
    let capped: String = rustia
        .run("local ok, err = pcall(table.create, 200000) return tostring(ok)")
        .unwrap();
    assert_eq!(capped, "false");

    // Aliased calls take the compiled (non-native) route with the same result.
    let aliased: f64 = rustia
        .run("local s = 'str' local tc = table.create local t = tc(5) return #t")
        .unwrap();
    assert_eq!(aliased, 0.0);
}

#[test]
fn puc51_compat_profile_codegen_is_used_by_hot_reload_and_precompiled_modules() {
    let mut rustia = RustiaBuilder::puc51_compat().build().unwrap();
    let source = "local t = {1, [1] = 9} return { slot = t[1] }";

    let reloaded = rustia.hot_reload("reload.luau", source).unwrap();
    assert_eq!(reloaded.get::<f64>(&mut rustia, "slot").unwrap(), 1.0);

    let module = rustia::Rustia::<()>::compile_module_with_profile(
        "precompiled.luau",
        source,
        rustia::Profile::Puc51Compat,
    )
    .unwrap();
    let installed = rustia.install(module).unwrap();
    assert_eq!(installed.get::<f64>(&mut rustia, "slot").unwrap(), 1.0);
}

#[test]
fn puc51_compat_sort_rejects_existing_slot_writes_from_comparator() {
    let mut rustia = RustiaBuilder::puc51_compat().build().unwrap();
    let rejected: bool = rustia
        .run(
            "local t = {3, 2, 1} \
             local ok, err = pcall(table.sort, t, function(a, b) \
                 t[1] = 99 \
                 return a < b \
             end) \
             return not ok and string.find(err, 'table modified during sorting', 1, true) ~= nil",
        )
        .unwrap();
    assert!(rejected);
}

#[test]
fn puc51_compat_sort_allows_comparator_writes_redirected_by_newindex() {
    let mut rustia = RustiaBuilder::puc51_compat().build().unwrap();
    let sorted: bool = rustia
        .run(
            "local redirected = {} \
             local t = setmetatable({3, 2, 1}, { \
                 __newindex = function(_, key, value) redirected[key] = value end \
             }) \
             table.sort(t, function(a, b) \
                 t[4] = 99 \
                 return a < b \
             end) \
             return t[1] == 1 and redirected[4] == 99",
        )
        .unwrap();
    assert!(sorted);
}
