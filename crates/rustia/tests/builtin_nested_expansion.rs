//! Pinned Luau 0.709 result counts across nested call and coroutine boundaries.
use rustia::{ErrorKind, Profile, RustiaBuilder};

fn nested(depth: usize, leaf: &str) -> String {
    let mut expression = leaf.to_owned();
    for _ in 0..depth {
        expression = format!("pass({expression})");
    }
    expression
}

#[test]
fn nested_trailing_calls_preserve_zero_and_many_results() {
    for depth in [1, 2, 3, 4, 8, 16] {
        let many = nested(depth, "many()");
        let empty = nested(depth, "empty()");
        let cases = [
            (format!("return tostring(select('#',{many}))"), Some("2")),
            (format!("string.len=function(...) return select('#',...) end; return tostring(0+string.len('abc',{many}))"), Some("3")),
            (format!("string.len=function(...) return select('#',...) end; return tostring((function() return string.len('abc',{many}) end)())"), Some("3")),
            (format!("string.len=function(...) return select('#',...) end; local t={{string.len('abc',{many})}}; return tostring(t[1])"), Some("3")),
            (format!("local o={{f=function(self,...) return ... end}}; return tostring(select('#',o:f({many})))"), Some("2")),
            (format!("return tostring(type({empty}))"), None),
            (format!("local t={{type({empty})}}; return tostring(t[1])"), None),
            (format!("local t={{}}; local ignored={{table.insert(t,1,{many})}}; return tostring(t[1])"), None),
            (format!("return tostring(select('#',({many})))"), Some("1")),
        ];
        for profile in [
            Profile::LuauCompat,
            Profile::ReproducibleStandard,
            Profile::DeterministicSim,
            Profile::Puc51Compat,
        ] {
            for (body, expected) in &cases {
                let source = format!("local function pass(...) return ... end; local function many() return 4,5 end; local function empty() end; {body}");
                let mut vm = RustiaBuilder::standard()
                    .profile(profile)
                    .memory_limit_bytes(2 * 1024 * 1024)
                    .build()
                    .unwrap();
                match expected {
                    Some(expected) => assert_eq!(
                        vm.eval::<String>(&source).unwrap(),
                        *expected,
                        "{profile:?} depth {depth}: {source}"
                    ),
                    None => assert!(
                        matches!(
                            vm.eval::<String>(&source).unwrap_err().kind(),
                            ErrorKind::Vm(_)
                        ),
                        "{profile:?} depth {depth}: {source}"
                    ),
                }
                assert_eq!(vm.eval::<i32>("return 1").unwrap(), 1);
            }
        }
    }
}

#[test]
fn nested_varargs_expand_in_scalar_return_and_constructor_calls() {
    for depth in [1, 2, 3, 4, 8, 16] {
        let values = nested(depth, "...");
        for body in [
            format!("return 0+string.len('abc',{values})"),
            format!("return string.len('abc',{values})"),
            format!("local t={{string.len('abc',{values})}}; return t[1]"),
            format!("local o={{f=function(self,...) return ... end}}; return o:f(string.len('abc',{values}))"),
        ] {
            let source = format!("local function pass(...) return ... end; string.len=function(...) return select('#',...) end; local function run(...) {body} end; return tostring(run(4,5))");
            let mut vm = RustiaBuilder::standard().build().unwrap();
            assert_eq!(vm.eval::<String>(&source).unwrap(), "3", "{source}");
        }
    }
}

#[test]
fn deep_expansion_can_suspend_in_arguments_and_selected_callees() {
    for depth in [3, 8, 16] {
        let values = nested(depth, "argument(...)");
        for body in [
            format!("return 0+string.len('abc',{values})"),
            format!("return string.len('abc',{values})"),
            format!("local t={{string.len('abc',{values})}}; return t[1]"),
        ] {
            let source = format!("local function pass(...) return ... end; local function argument(...) coroutine.yield('argument'); string.len=function(...) coroutine.yield('callee'); return select('#',...) end; return ... end; local co=coroutine.create(function(...) {body} end); local a,b=coroutine.resume(co,4,5); local c,d=coroutine.resume(co); local e,f=coroutine.resume(co); return tostring(a)..':'..tostring(b)..':'..tostring(c)..':'..tostring(d)..':'..tostring(e)..':'..tostring(f)");
            let mut vm = RustiaBuilder::standard()
                .memory_limit_bytes(2 * 1024 * 1024)
                .build()
                .unwrap();
            for _ in 0..2 {
                assert_eq!(
                    vm.eval::<String>(&source).unwrap(),
                    "true:argument:true:callee:true:3",
                    "{source}"
                );
                vm.reset_decision().unwrap();
            }
        }
    }
}

#[test]
fn recursive_expanded_calls_complete_and_resume_without_native_stack_growth() {
    for depth in [20, 100] {
        let source = format!("local function run(n) if n==0 then return 4,5 end; return string.len('abc',run(n-1)) end; string.len=function(...) return select('#',...),7 end; return tostring(run({depth}))");
        let mut vm = RustiaBuilder::standard()
            .memory_limit_bytes(2 * 1024 * 1024)
            .build()
            .unwrap();
        assert_eq!(vm.eval::<String>(&source).unwrap(), "3", "{source}");
    }
    let source = "local function run(n) if n==0 then coroutine.yield('base'); return 4,5 end; return string.len('abc',run(n-1)) end; string.len=function(...) return select('#',...),7 end; local co=coroutine.create(function() return run(20) end); local a,b=coroutine.resume(co); local c,d=coroutine.resume(co); return tostring(a)..':'..tostring(b)..':'..tostring(c)..':'..tostring(d)";
    let mut vm = RustiaBuilder::standard()
        .memory_limit_bytes(2 * 1024 * 1024)
        .build()
        .unwrap();
    assert_eq!(vm.eval::<String>(source).unwrap(), "true:base:true:3");
}
