use rustia::{ModuleSource, NewObject, ObjectType, Rustia, RustiaBuilder, ServiceModuleResolver};
use std::cell::Cell;
use std::rc::Rc;

fn module_vm(modules: &[(&'static str, &'static str)]) -> Rustia {
    let modules = modules.to_vec();
    let resolver = ServiceModuleResolver::from_fn(move |requested, _| {
        let source = modules
            .iter()
            .find_map(|(name, source)| (*name == requested).then_some(*source))
            .ok_or_else(|| rustia::Error::embed(format!("unknown module {requested}")))?;
        Ok(ModuleSource::new(requested, source))
    });
    RustiaBuilder::standard()
        .memory_limit_bytes(2 * 1024 * 1024)
        .module_resolver(resolver)
        .build()
        .unwrap()
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn nested_modules_resume_repeatedly_and_publish_only_completed_results() {
    let mut vm = module_vm(&[
        (
            "outer",
            "local inner = require('inner'); return { inner = inner }",
        ),
        (
            "inner",
            r#"
                _G.module_runs = (_G.module_runs or 0) + 1
                local first = coroutine.yield('first')
                local second = coroutine.yield(first + 1)
                return { answer = first + second }
            "#,
        ),
    ]);
    assert!(vm
        .load(
            r#"
                local owner = coroutine.create(function() return require('outer') end)
                local ok, value = coroutine.resume(owner)
                assert(ok and value == 'first')
                assert(not pcall(require, 'inner'))
                collectgarbage('collect')
                ok, value = coroutine.resume(owner, 20)
                assert(ok and value == 21)
                ok, value = coroutine.resume(owner, 22)
                assert(ok and value.inner.answer == 42)
                return coroutine.status(owner) == 'dead' and module_runs == 1
                    and require('outer') == value and require('inner') == value.inner
            "#,
        )
        .eval::<bool>()
        .unwrap());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn failed_suspended_modules_keep_completed_writes_and_allow_retry() {
    let mut vm = module_vm(&[(
        "bad",
        r#"
            _G.module_runs = (_G.module_runs or 0) + 1
            _G.before_yield = true
            coroutine.yield('waiting')
            error('module failed', 0)
        "#,
    )]);
    assert!(vm
        .load(
            r#"
                local owner = coroutine.create(function() return require('bad') end)
                local ok, value = coroutine.resume(owner)
                assert(ok and value == 'waiting')
                local other = coroutine.create(function() unrelated = 22 end)
                assert(coroutine.resume(other))
                ok, value = coroutine.resume(owner)
                assert(not ok and string.find(value, 'module failed', 1, true))
                assert(before_yield and module_runs == 1 and unrelated == 22)
                local retry = coroutine.create(function() return require('bad') end)
                ok, value = coroutine.resume(retry)
                assert(ok and value == 'waiting' and module_runs == 2)
                assert(coroutine.close(retry))
                return unrelated == 22
            "#,
        )
        .eval::<bool>()
        .unwrap());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn cycles_after_yield_release_every_loading_identity() {
    let mut vm = module_vm(&[
        ("a", "coroutine.yield('a'); return require('b')"),
        ("b", "return require('a')"),
    ]);
    assert!(vm
        .load(
            r#"
                for attempt = 1, 2 do
                    local owner = coroutine.create(function() return require('a') end)
                    local ok, value = coroutine.resume(owner)
                    assert(ok and value == 'a')
                    ok, value = coroutine.resume(owner)
                    assert(not ok and string.find(value, 'a -> b -> a', 1, true))
                end
                return true
            "#,
        )
        .eval::<bool>()
        .unwrap());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn closed_and_collected_owners_do_not_leave_modules_loading() {
    let mut vm = module_vm(&[("waiting", "coroutine.yield('waiting'); return 42")]);
    assert!(vm
        .load(
            r#"
                local owner = coroutine.create(function() return require('waiting') end)
                local ok, value = coroutine.resume(owner)
                assert(ok and value == 'waiting')
                assert(coroutine.close(owner))
                abandoned = coroutine.create(function() return require('waiting') end)
                ok, value = coroutine.resume(abandoned)
                assert(ok and value == 'waiting')
                abandoned = nil
                return true
            "#,
        )
        .eval::<bool>()
        .unwrap());
    assert!(vm
        .load(
            r#"
                collectgarbage('collect')
                collectgarbage('collect')
                local retry = coroutine.create(function() return require('waiting') end)
                local ok, value = coroutine.resume(retry)
                assert(ok and value == 'waiting')
                ok, value = coroutine.resume(retry)
                return ok and value == 42 and require('waiting') == 42
            "#,
        )
        .eval::<bool>()
        .unwrap());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn reset_discards_suspended_loaders_and_starts_a_fresh_cache() {
    let mut vm = module_vm(&[("waiting", "coroutine.yield('waiting'); return 42")]);
    let setup = r#"
        pending = coroutine.create(function() return require('waiting') end)
        local ok, value = coroutine.resume(pending)
        return ok and value == 'waiting'
    "#;
    assert!(vm.load(setup).eval::<bool>().unwrap());
    vm.reset_decision().unwrap();
    assert!(vm.load("return pending == nil").eval::<bool>().unwrap());
    assert!(vm.load(setup).eval::<bool>().unwrap());
    assert!(vm
        .load("local ok, value = coroutine.resume(pending); return ok and value == 42")
        .eval::<bool>()
        .unwrap());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn failed_install_restores_an_already_suspended_loader() {
    let mut vm = module_vm(&[("waiting", "coroutine.yield('waiting'); return 42")]);
    vm.load(
        r#"
            pending = coroutine.create(function() return require('waiting') end)
            local ok, value = coroutine.resume(pending)
            assert(ok and value == 'waiting')
        "#,
    )
    .exec()
    .unwrap();
    let globals = vm.globals().unwrap();
    let error = vm
        .load(
            r#"
                local ok, value = coroutine.resume(pending)
                assert(ok and value == 42)
                candidate_write = true
                error('candidate failed', 0)
            "#,
        )
        .environment(&globals)
        .install()
        .unwrap_err();
    assert!(error.to_string().contains("candidate failed"));
    assert!(vm
        .load(
            r#"
                assert(candidate_write == nil and coroutine.status(pending) == 'suspended')
                local ok, value = coroutine.resume(pending)
                return ok and value == 42 and require('waiting') == 42
            "#,
        )
        .eval::<bool>()
        .unwrap());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn rejected_top_level_yields_release_the_loading_identity() {
    let mut vm = module_vm(&[("waiting", "coroutine.yield('waiting'); return 42")]);
    assert!(vm.load("return require('waiting')").eval::<i64>().is_err());
    assert!(vm
        .load(
            r#"
                local retry = coroutine.create(function() return require('waiting') end)
                local ok, value = coroutine.resume(retry)
                assert(ok and value == 'waiting')
                ok, value = coroutine.resume(retry)
                return ok and value == 42
            "#,
        )
        .eval::<bool>()
        .unwrap());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn fuel_abort_does_not_leave_a_module_loading() {
    let resolutions = Rc::new(Cell::new(0));
    let count = Rc::clone(&resolutions);
    let resolver = ServiceModuleResolver::from_fn(move |_, _| {
        let attempt = count.get();
        count.set(attempt + 1);
        Ok(ModuleSource::new(
            "retry",
            if attempt == 0 {
                "while true do end"
            } else {
                "return 42"
            },
        ))
    });
    let mut vm = RustiaBuilder::standard()
        .fuel(100)
        .module_resolver(resolver)
        .build()
        .unwrap();
    let error = vm
        .load("local co = coroutine.create(function() return require('retry') end); coroutine.resume(co)")
        .exec()
        .unwrap_err();
    assert!(matches!(
        error.as_vm_error().unwrap().kind(),
        rustia::VmErrorKind::FuelExhausted { limit: 100 }
    ));
    assert_eq!(
        vm.load("return require('retry')").eval::<i64>().unwrap(),
        42
    );
    assert_eq!(resolutions.get(), 2);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn suspended_modules_keep_object_aliases_alive_until_completion_and_reset() {
    struct Actor(i64);
    let mut vm = module_vm(&[(
        "actor",
        r#"
            local actor = make_actor()
            coroutine.yield('waiting')
            return {actor = actor, alias = actor}
        "#,
    )]);
    vm.register_object_type(ObjectType::new("Actor").method("score", 1, |a: &Actor| a.0))
        .unwrap();
    vm.register_function("make_actor", || NewObject::new(Actor(42)))
        .unwrap();
    let retained = vm
        .load(
            r#"
                local owner = coroutine.create(function() return require('actor') end)
                local ok, value = coroutine.resume(owner)
                assert(ok and value == 'waiting')
                for i = 1, 48 do local garbage = table.create(16000, i) end
                collectgarbage('collect')
                ok, value = coroutine.resume(owner)
                assert(ok and value.actor == value.alias and value.actor:score() == 42)
                collectgarbage('collect')
                assert(require('actor') == value and require('actor').alias:score() == 42)
                return value.actor
            "#,
        )
        .eval_live()
        .unwrap();
    assert!(retained.as_object::<Actor>(&mut vm).unwrap().is_some());
    vm.reset_decision().unwrap();
    assert!(retained.as_object::<Actor>(&mut vm).is_err());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn yielded_empty_and_nil_results_keep_their_distinct_cached_arities() {
    let mut vm = module_vm(&[
        (
            "empty",
            "_G.empty_runs = (_G.empty_runs or 0) + 1; coroutine.yield('empty'); return",
        ),
        (
            "nil",
            "_G.nil_runs = (_G.nil_runs or 0) + 1; coroutine.yield('nil'); return nil",
        ),
    ]);
    assert!(vm
        .load(
            r#"
                local empty = coroutine.create(function() return select('#', require('empty')) end)
                local nothing = coroutine.create(function() return select('#', require('nil')) end)
                local ok, value = coroutine.resume(empty)
                assert(ok and value == 'empty')
                ok, value = coroutine.resume(nothing)
                assert(ok and value == 'nil')
                ok, value = coroutine.resume(empty)
                assert(ok and value == 0)
                ok, value = coroutine.resume(nothing)
                return ok and value == 1 and select('#', require('empty')) == 0
                    and select('#', require('nil')) == 1 and empty_runs == 1 and nil_runs == 1
            "#,
        )
        .eval::<bool>()
        .unwrap());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn yielded_multiple_results_fail_without_caching_or_leaving_ownership() {
    let mut vm = module_vm(&[(
        "many",
        "_G.many_runs = (_G.many_runs or 0) + 1; coroutine.yield('many'); return 1, 2",
    )]);
    assert!(vm
        .load(
            r#"
                for attempt = 1, 2 do
                    local owner = coroutine.create(function() return require('many') end)
                    local ok, value = coroutine.resume(owner)
                    assert(ok and value == 'many')
                    ok, value = coroutine.resume(owner)
                    assert(not ok and string.find(value, 'module must return a single value', 1, true))
                end
                return many_runs == 2
            "#,
        )
        .eval::<bool>()
        .unwrap());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn independent_suspended_modules_can_complete_in_reverse_loading_order() {
    let mut vm = module_vm(&[
        (
            "left",
            "local value = coroutine.yield('left'); return {value = value}",
        ),
        (
            "right",
            "local value = coroutine.yield('right'); return {value = value}",
        ),
    ]);
    assert!(vm
        .load(
            r#"
                local left = coroutine.create(function() return require('left') end)
                local right = coroutine.create(function() return require('right') end)
                local ok, value = coroutine.resume(left)
                assert(ok and value == 'left')
                ok, value = coroutine.resume(right)
                assert(ok and value == 'right')
                local right_ok, right_value = coroutine.resume(right, 23)
                assert(right_ok and right_value.value == 23)
                assert(not pcall(require, 'left'))
                local left_ok, left_value = coroutine.resume(left, 19)
                return left_ok and left_value.value + right_value.value == 42
                    and require('left') == left_value and require('right') == right_value
            "#,
        )
        .eval::<bool>()
        .unwrap());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn required_modules_can_yield_through_expanded_argument_and_return_shapes() {
    for body in [
        "return pass(pass(require(path())))",
        "local value = pass(pass(require(path()))); return value",
        "return {pass(pass(require(path())))}",
        "return box:call(pass(require(path())))",
        "return {box:call(pass(require(path())))}",
        "local first, second = pass(0, require(path())); return first + second",
        "local function indirect(...) return pass(require(...)) end; return indirect('wrapped')",
        "return pass(pass(pass(pass(pass(require(path()))))))",
    ] {
        let mut vm = module_vm(&[(
            "wrapped",
            "local value = coroutine.yield('waiting'); return value",
        )]);
        let source = format!(
            r#"
                local function pass(...) return ... end
                local function path() return 'wrapped' end
                local box = {{}}
                function box:call(...) return ... end
                local function work(...)
                    {body}
                end
                local owner = coroutine.create(function() return work('wrapped') end)
                local ok, value = coroutine.resume(owner)
                assert(ok and value == 'waiting')
                ok, value = coroutine.resume(owner, 7)
                if type(value) == 'table' then value = value[1] end
                return ok and value == 7
            "#,
        );
        let result = vm.load(&source).eval::<bool>().unwrap_or_else(|error| {
            panic!("{body}: {}", error.report());
        });
        assert!(result, "{body}");
    }
}
