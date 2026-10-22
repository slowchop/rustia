//! Exact guest-visible messages captured from the pinned Luau 0.709 oracle.
use rustia::{Profile, RustiaBuilder};

#[test]
fn forbidden_metamethod_yields_have_no_source_prefix_and_leave_dead_coroutines() {
    let bodies = [
        "local t=setmetatable({}, {__index=yielding}); return t.x",
        "local t=setmetatable({}, {__index=yielding}); return t:x()",
        "local t=setmetatable({}, {__newindex=yielding}); t.x=1",
        "local t=setmetatable({}, {__add=yielding}); return t+1",
        "local t=setmetatable({}, {__len=yielding}); return #t",
        "local t=setmetatable({}, {__tostring=yielding}); return tostring(t)",
        "table.sort({2,1}, yielding)",
        "local t=setmetatable({}, {__iter=yielding}); for k in t do end",
        "local t=setmetatable({}, {__concat=yielding}); return t..t",
        "local mt={__eq=yielding}; return setmetatable({},mt)==setmetatable({},mt)",
    ];
    for profile in [
        Profile::LuauCompat,
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::Puc51Compat,
    ] {
        let mut vm = RustiaBuilder::standard()
            .profile(profile)
            .fuel(100_000)
            .memory_limit_bytes(8 * 1024 * 1024)
            .build()
            .unwrap();
        for body in bodies {
            let source = format!(
                "local function yielding() return coroutine.yield('pause') end\n\
                 local c=coroutine.create(function() {body} end)\n\
                 local a,b=coroutine.resume(c); local d,e=coroutine.resume(c);\n\
                 return tostring(a)..':'..tostring(b)..':'..tostring(d)..':'..tostring(e)"
            );
            let actual: String = vm
                .load(&source)
                .source_name("boundary.luau")
                .eval()
                .unwrap();
            assert_eq!(
                actual,
                "false:attempt to yield across metamethod/C-call boundary:false:cannot resume dead coroutine",
                "{profile:?}: {body}"
            );
            // A rejected yield must not leave the VM in a suspended execution.
            assert_eq!(vm.eval::<i32>("return 6*7").unwrap(), 42);
        }
    }
}
