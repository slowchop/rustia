//! Pinned Luau 0.709 distinguishes recognized builtin lookup from ordinary calls.
use rustia::{Profile, RustiaBuilder};

#[test]
fn builtin_callees_follow_pinned_argument_evaluation_order() {
    let cases = [
        (
            "local function extra() string.len=function() return 100 end; return 99 end; return ''..string.len('abc',extra())",
            "100",
        ),
        (
            "local function extra() string.len=function() return 100 end; return 99 end; local v=string.len('abc',extra()); return ''..v",
            "100",
        ),
        (
            "local function extra() string.len=function() return 100 end; return 99 end; return ''..(string.len)('abc',extra())",
            "3",
        ),
        (
            "local function extra() string.len=function() return 100 end; return 99 end; return ''..string.len('abc',(extra()))",
            "100",
        ),
        (
            "local len=string.len; local function extra() string.len=function() return 100 end; return 99 end; return ''..len('abc',extra())",
            "3",
        ),
        (
            "local len=string.len; local function extra() len=function() return 100 end; return 99 end; return ''..len('abc',extra())",
            "3",
        ),
        (
            "string.len=function() return 10 end; local function extra() string.len=function() return 100 end; return 99 end; return ''..string.len('abc',extra())",
            "100",
        ),
        (
            "local function extra() string={len=function() return 100 end}; return 99 end; return ''..string.len('abc',extra())",
            "3",
        ),
        (
            "local function get() return string end; local function extra() string.len=function() return 100 end; return 99 end; return ''..get().len('abc',extra())",
            "3",
        ),
        (
            "string.unreal=function() return 10 end; local function extra() string.unreal=function() return 100 end; return 99 end; return ''..string.unreal('abc',extra())",
            "10",
        ),
        (
            "local function extra() tostring=function() return 'replacement' end; return 99 end; return 'value:'..tostring(1,extra())",
            "value:1",
        ),
        (
            "local function extra() tostring=function() return 'replacement' end; return 99 end; return 'value:'..(tostring)(1,extra())",
            "value:1",
        ),
        (
            "local f=tostring; local function extra() tostring=function() return 'replacement' end; return 99 end; return 'value:'..f(1,extra())",
            "value:1",
        ),
        (
            "local function extra() string.len=function() return 100 end; return 'abc' end; return ''..string.len((extra()))",
            "100",
        ),
        (
            "local function extra() math.abs=function() return 100 end; return -3 end; return ''..math.abs((extra()))",
            "100",
        ),
        (
            "local function extra() type=function() return 'replacement' end; return 3 end; return ''..type((extra()))",
            "number",
        ),
        (
            "getfenv(0); local function extra() string.len=function() return 100 end; return 99 end; return ''..string.len('abc',extra())",
            "100",
        ),
        (
            "local function extra() string.len=function() return 100 end; return 99 end; local function run() return string.len('abc',extra()) end; return ''..run()",
            "100",
        ),
        (
            "local g=_G; local function extra() g.tostring=function() return 'replacement' end; return 99 end; return 'value:'..tostring(1,extra())",
            "value:replacement",
        ),
        (
            "local s=string; local function extra() string.len=function() return 100 end; return 99 end; return ''..s.len('abc',extra())",
            "3",
        ),
        (
            "string.len=function() return 10 end; local f=string.len; local function extra() string.len=function() return 100 end; return 99 end; return ''..f('abc',extra())",
            "10",
        ),
        (
            "tostring=function() return 'first' end; local g=_G; local function extra() g.tostring=function() return 'replacement' end; return 99 end; return 'value:'..tostring(1,extra())",
            "value:first",
        ),
        (
            "local function extra() string.upper=function() return 'replacement' end; return 99 end; return ''..string.upper('abc',extra())",
            "ABC",
        ),
        (
            "local value=setmetatable({}, {__add=function() math.abs=function() return 100 end; return -3 end}); return ''..math.abs(value+0)",
            "100",
        ),
        (
            "local value=setmetatable({}, {__index=function() string.len=function() return 100 end; return 'abc' end}); return ''..string.len(value.text)",
            "100",
        ),
        (
            "local value=setmetatable({}, {__add=function() math.abs=function() return 100 end; return -3 end}); return ''..(math.abs)(value+0)",
            "3",
        ),
        (
            "local g=_G; local function extra() g.tostring=function() return 'second' end; return 99 end; local result=tostring(1,extra()); tostring=function() return 'later' end; return result",
            "1",
        ),
        (
            "rawset(_G,'tostring',function() return 'first' end); local g=_G; local function extra() g.tostring=function() return 'second' end; return 99 end; return tostring(1,extra())",
            "second",
        ),
    ];
    for (source, expected) in cases {
        let mut vm = RustiaBuilder::standard()
            .profile(Profile::LuauCompat)
            .build()
            .unwrap();
        assert_eq!(vm.eval::<String>(source).unwrap(), expected, "{source}");
    }
}

#[test]
fn deferred_builtin_calls_can_suspend_in_arguments_and_again_in_the_callee() {
    for body in [
        "return 0 + string.len('abc', argument(99,88))",
        "local value = string.len('abc', argument(99,88)); return value",
        "return string.len('abc', argument(99,88))",
        "local values = {string.len('abc', argument(99,88))}; return values[1]",
        "return 0 + string.len('abc', pass(argument(99,88)))",
        "return string.len('abc', pass(argument(99,88)))",
        "return 0 + string.len('abc', pass(pass(argument(99,88))))",
        "local object={pass=function(self,...) return ... end}; return object:pass(string.len('abc',argument(99,88)))",
        "local object={pass=function(self,...) return ... end}; return 0 + object:pass(string.len('abc',argument(99,88)))",
        "return 0 + string.len('abc', argument(...))",
        "return string.len('abc', argument(...))",
        "return string.len('abc', pass(argument(...)))",
    ] {
        let source = format!(
            "local function argument(...) \
                coroutine.yield('argument'); \
                string.len = function(...) coroutine.yield('callee'); return select('#', ...) end; \
                return ... \
             end; \
             local function pass(...) return ... end; \
             local thread=coroutine.create(function(...) {body} end); \
             local a,b=coroutine.resume(thread,99,88); \
             local c,d=coroutine.resume(thread); \
             local e,f=coroutine.resume(thread); \
             return tostring(a)..':'..tostring(b)..':'..tostring(c)..':'..tostring(d)..':'..tostring(e)..':'..tostring(f)"
        );
        let mut vm = RustiaBuilder::standard()
            .profile(Profile::LuauCompat)
            .memory_limit_bytes(2 * 1024 * 1024)
            .build()
            .unwrap();
        assert_eq!(
            vm.eval::<String>(&source).unwrap(),
            "true:argument:true:callee:true:3",
            "{body}"
        );
        vm.reset_decision().unwrap();
        assert_eq!(
            vm.eval::<String>(&source).unwrap(),
            "true:argument:true:callee:true:3",
            "after reset: {body}"
        );
    }
}
