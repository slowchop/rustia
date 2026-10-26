use rustia::{ModuleSource, RustiaBuilder, ServiceModuleResolver, VmErrorKind};

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn ordinary_callable_recursion_uses_vm_frames() -> rustia::Result<()> {
    let mut vm = RustiaBuilder::standard()
        .fuel(100_000)
        .recursion_limit(512)
        .memory_limit_bytes(2 * 1024 * 1024)
        .build()?;
    for depth in [20, 40, 100] {
        vm.insert_global("depth", depth)?;
        let result: i64 = vm
            .load(
                r#"
                local left, right = {}, {}
                setmetatable(left, {__call = function(self, n)
                    assert(self == left)
                    if n == 0 then return 0 end
                    return 1 + right(n - 1)
                end})
                setmetatable(right, {__call = function(self, n)
                    assert(self == right)
                    if n == 0 then return 0 end
                    return 1 + left(n - 1)
                end})
                return left(depth)
            "#,
            )
            .eval()?;
        assert_eq!(result, depth);
    }
    Ok(())
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn callable_receivers_survive_method_and_module_yields() -> rustia::Result<()> {
    let resolver = ServiceModuleResolver::from_fn(|_, _| {
        Ok(ModuleSource::new(
            "waiting",
            "local answer = coroutine.yield('module'); return {answer = answer}",
        ))
    });
    let mut vm = RustiaBuilder::standard()
        .fuel(100_000)
        .memory_limit_bytes(2 * 1024 * 1024)
        .module_resolver(resolver)
        .build()?;
    assert!(vm
        .load(
            r#"
            local outer = {}
            setmetatable(outer, {__call = function(first, name)
                assert(first == outer and name == 'waiting')
                coroutine.yield('receiver')
                return require(name)
            end})
            local owner = coroutine.create(function() return outer('waiting') end)
            local ok, value = coroutine.resume(owner)
            assert(ok and value == 'receiver')
            collectgarbage('collect')
            ok, value = coroutine.resume(owner)
            assert(ok and value == 'module')
            collectgarbage('collect')
            ok, value = coroutine.resume(owner, 42)
            return ok and value.answer == 42 and require('waiting') == value
        "#,
        )
        .eval::<bool>()?);
    Ok(())
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn recursive_callable_function_reports_configured_recursion_exhaustion() -> rustia::Result<()> {
    let mut vm = RustiaBuilder::standard().recursion_limit(16).build()?;
    let error = vm.load(
        "local cycle = {}; setmetatable(cycle, {__call = function(self) return self() end}); pcall(function() cycle() end)",
    ).exec().unwrap_err();
    assert!(matches!(
        error.as_vm_error().unwrap().kind(),
        VmErrorKind::RecursionLimit { .. }
    ));
    assert_eq!(vm.load("return 42").eval::<i64>()?, 42);
    Ok(())
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn call_metamethod_must_be_a_function_in_all_call_shapes() -> rustia::Result<()> {
    let mut vm = RustiaBuilder::standard().build()?;
    assert!(vm
        .load(
            r#"
        local inner = setmetatable({}, {__call = function() return true end})
        local outer = setmetatable({}, {__call = inner})
        local function arguments() return 1, 2 end
        for _, value in ipairs({false, 42, 'call', inner, outer}) do
            local receiver = setmetatable({}, {__call = value})
            local ok, err = pcall(function() return receiver() end)
            assert(not ok and string.find(err, 'attempt to call a table value', 1, true))
            ok, err = pcall(function() return receiver(arguments()) end)
            assert(not ok and string.find(err, 'attempt to call a table value', 1, true))
        end
        return true
    "#
        )
        .eval::<bool>()?);
    Ok(())
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn resumed_callers_observe_mutated_capture_cells() -> rustia::Result<()> {
    for metered in [false, true] {
        let builder = RustiaBuilder::standard();
        let mut vm = if metered {
            builder.fuel(100_000)
        } else {
            builder
        }
        .build()?;
        assert!(vm
            .load(
                r#"
            local owner = coroutine.create(function()
                local captured = 1
                local function set(value) captured = value end
                local child = setmetatable({}, {__call = function()
                    coroutine.yield(set)
                    assert(captured == 20)
                    set(42)
                    return 7
                end})
                local result = child()
                return captured == 42 and result == 7
            end)
            local ok, set = coroutine.resume(owner)
            assert(ok and type(set) == 'function')
            set(20)
            collectgarbage('collect')
            local done, result = coroutine.resume(owner)
            return done and result and coroutine.status(owner) == 'dead'
        "#
            )
            .eval::<bool>()?);
    }
    Ok(())
}
