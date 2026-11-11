//! `error(message, level)` must report the selected caller's line whatever
//! shape that caller's pending call has. A call whose results are left open
//! (the last item of a table constructor, the last argument of another call,
//! a `return f()` tail) is compiled differently from a fixed-result call.
use rustia::{Profile, RustiaBuilder};

fn profiles() -> [Profile; 4] {
    [
        Profile::LuauCompat,
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::Puc51Compat,
    ]
}

/// `f` is two frames above `error`, so level 3 selects the line that calls `f`.
const PRELUDE: &str = "\
local function fail(message)
    error(message, 3)
end
function f()
    fail('bad')
end
function g(...)
    return ...
end";

fn message_for(profile: Profile, user: &str) -> String {
    let mut vm = RustiaBuilder::standard().profile(profile).build().unwrap();
    vm.load(PRELUDE).source_name("prelude.luau").exec().unwrap();
    let source = format!("local ok, e = pcall(function()\n{user}\nend)\nreturn e");
    vm.load(&source).source_name("user.luau").eval().unwrap()
}

#[test]
fn error_level_reports_the_caller_line_for_every_call_shape() {
    let cases = [
        ("last table item", "local t = { f() }"),
        ("only item of a table call argument", "local t = g{ f() }"),
        ("last table item after others", "local t = { 1, 2, f() }"),
        ("middle table item", "local t = { 1, f(), 2 }"),
        ("last call argument", "local v = g(f())"),
        ("fixed-result call argument", "local v = g(f(), 1)"),
        ("statement call", "f()"),
        ("local assignment", "local v = f()"),
        (
            "vararg call as last table item",
            "local t = (function(...) return { f(...) } end)(1)",
        ),
        ("open return", "local v = (function() return 1, f() end)()"),
        (
            "vararg open return",
            "local v = (function(...) return f(...) end)(1)",
        ),
        (
            "vararg open return after a value",
            "local v = (function(...) return 1, f(...) end)(1)",
        ),
    ];
    for profile in profiles() {
        for (name, user) in cases {
            assert_eq!(
                message_for(profile, user),
                "user.luau:2: bad",
                "{profile:?}: {name}: {user}"
            );
        }
    }
}

#[test]
fn uncaught_error_level_reports_the_caller_line_for_a_last_table_item() {
    for profile in profiles() {
        let mut vm = RustiaBuilder::standard().profile(profile).build().unwrap();
        vm.load(PRELUDE).source_name("prelude.luau").exec().unwrap();
        let error = vm
            .load("-- user marker\nlocal t = { f() }")
            .source_name("user.luau")
            .exec()
            .unwrap_err();
        assert!(
            error.to_string().contains("user.luau:2:"),
            "{profile:?}: {error}"
        );
    }
}
