//! Captured with the pinned Luau 0.709 worker; see design/82.
use rustia::{ErrorKind, Profile, RustiaBuilder};

#[test]
fn builtin_argument_lists_preserve_zero_one_and_many_results() {
    let cases = [
        (
            "local t={}; local function many() return 4,99 end; table.insert(t,1,many()); return 'wrong'",
            None,
        ),
        (
            "local t={}; local function many() return 4,99 end; table.insert(t,1,(many())); return ''..t[1]",
            Some("4"),
        ),
        (
            "local t={}; local function empty() end; table.insert(t,1,4,empty()); return ''..t[1]",
            Some("4"),
        ),
        (
            "local function empty() end; return 'x'..type(empty())",
            None,
        ),
        (
            "local function empty() end; return 'x'..typeof(empty())",
            None,
        ),
        (
            "local function empty() end; return 'x'..type((empty()))",
            Some("xnil"),
        ),
        (
            "local function empty() end; return 'x'..type((empty()), 4)",
            Some("xnil"),
        ),
        (
            "local function empty() end; return 'x'..tonumber(empty())",
            None,
        ),
        (
            "local function empty() end; return rawget({},empty())",
            None,
        ),
        (
            "local function empty() end; return 'x'..tostring(rawget({},(empty())))",
            Some("xnil"),
        ),
        (
            "local function empty() end; return rawset({},'key',empty())",
            None,
        ),
        (
            "local function args() return 2,7 end; return ''..table.create(args())[2]",
            Some("7"),
        ),
        (
            "local function args() return 'abcd',2,3 end; return 'x'..string.sub(args())",
            Some("xbc"),
        ),
        (
            "local function args() return '101',2 end; return ''..tonumber(args())",
            Some("5"),
        ),
        (
            "local function many() return 4,99 end; return 'x'..type(many())",
            Some("xnumber"),
        ),
        (
            "local object={}; function object:empty() end; return 'x'..type(object:empty())",
            None,
        ),
        (
            "local function run(...) return 'x'..type(...) end; return run()",
            None,
        ),
        (
            "local function run(...) local t={}; table.insert(t,1,...); return ''..t[1] end; return run(4,99)",
            None,
        ),
        (
            "local function run(...) local t={}; table.insert(t,1,(...)); return ''..t[1] end; return run(4,99)",
            Some("4"),
        ),
        (
            "local function many() return 4,99 end; local t={assert(many())}; return #t..':'..t[1]..':'..t[2]",
            Some("2:4:99"),
        ),
        (
            "local function many() return 4,99 end; return 'x'..select('#',assert(many()))",
            Some("x2"),
        ),
    ];
    for profile in [
        Profile::LuauCompat,
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::Puc51Compat,
    ] {
        let mut vm = RustiaBuilder::standard().profile(profile).build().unwrap();
        for (body, expected) in cases {
            vm.insert_global("observed", 0).unwrap();
            let source = format!("observed=1; {body}");
            let result = vm.eval::<String>(&source);
            match expected {
                Some(value) => assert_eq!(result.unwrap(), value, "{profile:?}: {body}"),
                None => {
                    let error = result.unwrap_err();
                    assert!(
                        matches!(error.kind(), ErrorKind::Vm(_)),
                        "{profile:?}: {body}: {error}"
                    );
                }
            }
            assert_eq!(vm.eval::<i32>("return observed").unwrap(), 1, "{body}");
        }
    }
}
