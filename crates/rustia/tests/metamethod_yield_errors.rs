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
        "return string.gsub('a','.',yielding)",
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

#[test]
fn synchronous_native_callbacks_reject_yield_inside_the_protected_call() {
    let sources = [
        r#"
            local function nested()
                local seen
                local child=coroutine.create(function()
                    seen=coroutine.isyieldable()
                    coroutine.yield('ok')
                    return 'done'
                end)
                local a,b=coroutine.resume(child)
                local c,d=coroutine.resume(child)
                local failing=coroutine.create(function() error('child failure',0) end)
                local e,f=coroutine.resume(failing)
                assert(seen and a and b=='ok' and c and d=='done' and not e and f=='child failure')
            end
            local seen,ok,err
            local t=setmetatable({}, {__tostring=function()
                nested()
                seen=coroutine.isyieldable()
                ok,err=pcall(coroutine.yield,'pause')
                return 'value'
            end})
            tostring(t)
            return tostring(seen)..':'..tostring(ok)..':'..tostring(err)
        "#,
        r#"
            local function nested()
                local seen
                local child=coroutine.create(function()
                    seen=coroutine.isyieldable()
                    coroutine.yield('ok')
                    return 'done'
                end)
                local a,b=coroutine.resume(child)
                local c,d=coroutine.resume(child)
                local failing=coroutine.create(function() error('child failure',0) end)
                local e,f=coroutine.resume(failing)
                assert(seen and a and b=='ok' and c and d=='done' and not e and f=='child failure')
            end
            local seen,ok,err
            string.gsub('a','.',function()
                nested()
                seen=coroutine.isyieldable()
                ok,err=pcall(coroutine.yield,'pause')
                return 'b'
            end)
            return tostring(seen)..':'..tostring(ok)..':'..tostring(err)
        "#,
        r#"
            local seen,ok,err
            table.foreach({key='value'},function()
                seen=coroutine.isyieldable()
                ok,err=xpcall(coroutine.yield,function(message) return message end,'pause')
            end)
            return tostring(seen)..':'..tostring(ok)..':'..tostring(err)
        "#,
        r#"
            local seen,ok,err
            table.foreachi({'value'},function()
                seen=coroutine.isyieldable()
                ok,err=pcall(coroutine.yield,'pause')
            end)
            return tostring(seen)..':'..tostring(ok)..':'..tostring(err)
        "#,
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
        for source in sources {
            let actual: String = vm.load(source).eval().unwrap();
            assert_eq!(
                actual, "false:false:attempt to yield across metamethod/C-call boundary",
                "{profile:?}: {source}"
            );
        }
    }
}
