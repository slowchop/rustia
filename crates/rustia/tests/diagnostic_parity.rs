//! Pinned Luau diagnostics; see conformance/upstream/diagnostic_parity.md.
use rustia::{Profile, RustiaBuilder};

// Expected strings retain the complete source name and line.
const CASES: &[(&str, &str, &str)] = &[
    (
        "length_nil",
        r#"return #nil"#,
        r#"false:string:diagnostic_parity.luau:1: attempt to get length of a nil value"#,
    ),
    (
        "rawget_missing",
        r#"return rawget({})"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #2"#,
    ),
    (
        "rawset_missing",
        r#"return rawset({},1)"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #3"#,
    ),
    (
        "type_missing",
        r#"return type()"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #1"#,
    ),
    (
        "string_len_type",
        r#"return string.len({})"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'len' (string expected, got table)"#,
    ),
    (
        "string_sub_missing",
        r#"return string.sub("a")"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #2 to 'sub' (number expected)"#,
    ),
    (
        "insert_many",
        r#"return table.insert({},1,2,3)"#,
        r#"false:string:diagnostic_parity.luau:1: wrong number of arguments to 'insert'"#,
    ),
    (
        "sort_type",
        r#"return table.sort({},true)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #2 to 'sort' (function expected, got boolean)"#,
    ),
    (
        "next_key",
        r#"return next({},1)"#,
        r#"false:string:invalid key to 'next'"#,
    ),
    (
        "coroutine_resume_type",
        r#"return coroutine.resume({})"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'resume' (thread expected, got table)"#,
    ),
    (
        "coroutine_create_type",
        r#"return coroutine.create({})"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'create' (function expected, got table)"#,
    ),
    (
        "freeze_twice",
        r#"local t=table.freeze({}); return table.freeze(t)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'freeze' (table is already frozen)"#,
    ),
    (
        "freeze_locked",
        r#"return table.freeze(setmetatable({}, {__metatable=false}))"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'freeze' (table has a protected metatable)"#,
    ),
    (
        "rawget_absent",
        r#"return rawget()"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #1 to 'rawget' (table expected)"#,
    ),
    (
        "rawget_bad_first",
        r#"return rawget(false)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'rawget' (table expected, got boolean)"#,
    ),
    (
        "rawget_nil_first",
        r#"return rawget(nil,1)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'rawget' (table expected, got nil)"#,
    ),
    (
        "rawget_nil_key",
        r#"return rawget({},nil)"#,
        r#"true:nil:nil"#,
    ),
    (
        "rawset_absent",
        r#"return rawset()"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #1 to 'rawset' (table expected)"#,
    ),
    (
        "rawset_bad_first",
        r#"return rawset(false)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'rawset' (table expected, got boolean)"#,
    ),
    (
        "rawset_missing_key",
        r#"return rawset({})"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #2"#,
    ),
    (
        "rawset_nil_key_missing_value",
        r#"return rawset({},nil)"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #3"#,
    ),
    (
        "rawset_nil_key",
        r#"return rawset({},nil,1)"#,
        r#"false:string:table index is nil"#,
    ),
    (
        "rawset_nil_value",
        r#"local t={a=1}; rawset(t,"a",nil); return t.a"#,
        r#"true:nil:nil"#,
    ),
    (
        "rawset_readonly_missing_value",
        r#"return rawset(table.freeze({}),1)"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #3"#,
    ),
    (
        "rawset_readonly_nil_key",
        r#"return rawset(table.freeze({}),nil,1)"#,
        r#"false:string:attempt to modify a readonly table"#,
    ),
    ("type_nil", r#"return type(nil)"#, r#"true:string:nil"#),
    ("type_extra", r#"return type(1,2)"#, r#"true:string:number"#),
    (
        "len_missing",
        r#"return string.len()"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #1 to 'len' (string expected)"#,
    ),
    (
        "len_nil",
        r#"return string.len(nil)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'len' (string expected, got nil)"#,
    ),
    (
        "len_number",
        r#"return string.len(12345)"#,
        r#"true:number:5"#,
    ),
    (
        "len_extra",
        r#"return string.len("abc", false)"#,
        r#"true:number:3"#,
    ),
    (
        "sub_absent",
        r#"return string.sub()"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #1 to 'sub' (string expected)"#,
    ),
    (
        "sub_bad_first_missing_start",
        r#"return string.sub({})"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'sub' (string expected, got table)"#,
    ),
    (
        "sub_nil_start",
        r#"return string.sub("abc",nil)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #2 to 'sub' (number expected, got nil)"#,
    ),
    (
        "sub_bad_end",
        r#"return string.sub("abc",1,false)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #3 to 'sub' (number expected, got boolean)"#,
    ),
    (
        "sub_nil_end",
        r#"return string.sub("abc",2,nil)"#,
        r#"true:string:bc"#,
    ),
    (
        "sub_number_and_numeric_strings",
        r#"return string.sub(12345,"2","3")"#,
        r#"true:string:23"#,
    ),
    (
        "sub_extra",
        r#"return string.sub("abcd",2,3,false)"#,
        r#"true:string:bc"#,
    ),
    (
        "insert_absent",
        r#"return table.insert()"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #1 to 'insert' (table expected)"#,
    ),
    (
        "insert_one",
        r#"return table.insert({})"#,
        r#"false:string:diagnostic_parity.luau:1: wrong number of arguments to 'insert'"#,
    ),
    (
        "insert_bad_first_many",
        r#"return table.insert(false,1,2,3)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'insert' (table expected, got boolean)"#,
    ),
    (
        "insert_readonly_many",
        r#"return table.insert(table.freeze({}),1,2,3)"#,
        r#"false:string:diagnostic_parity.luau:1: wrong number of arguments to 'insert'"#,
    ),
    (
        "insert_readonly_bad_position",
        r#"return table.insert(table.freeze({}),false,1)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #2 to 'insert' (number expected, got boolean)"#,
    ),
    (
        "insert_nil_value",
        r#"local t={}; table.insert(t,nil); return #t"#,
        r#"true:number:0"#,
    ),
    (
        "insert_numeric_position",
        r#"local t={1,3}; table.insert(t,"2",2); return table.concat(t,",")"#,
        r#"true:string:1,2,3"#,
    ),
    (
        "sort_absent",
        r#"return table.sort()"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #1 to 'sort' (table expected)"#,
    ),
    (
        "sort_bad_first",
        r#"return table.sort(false,true)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'sort' (table expected, got boolean)"#,
    ),
    (
        "sort_readonly_bad_comparator",
        r#"return table.sort(table.freeze({}),true)"#,
        r#"false:string:attempt to modify a readonly table"#,
    ),
    (
        "sort_nil_comparator",
        r#"local t={2,1}; table.sort(t,nil); return table.concat(t,",")"#,
        r#"true:string:1,2"#,
    ),
    (
        "sort_empty_bad_comparator",
        r#"return table.sort({},true)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #2 to 'sort' (function expected, got boolean)"#,
    ),
    (
        "sort_callable_comparator",
        r#"return table.sort({},setmetatable({}, {__call=function() return true end}))"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #2 to 'sort' (function expected, got table)"#,
    ),
    (
        "next_absent",
        r#"return next()"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #1 to 'next' (table expected)"#,
    ),
    (
        "next_bad_first",
        r#"return next(false,1)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'next' (table expected, got boolean)"#,
    ),
    ("next_nil_key", r#"return next({},nil)"#, r#"true:nil:nil"#),
    (
        "create_absent",
        r#"return coroutine.create()"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #1 to 'create' (function expected)"#,
    ),
    (
        "create_nil",
        r#"return coroutine.create(nil)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'create' (function expected, got nil)"#,
    ),
    (
        "create_callable",
        r#"return coroutine.create(setmetatable({}, {__call=function() end}))"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'create' (function expected, got table)"#,
    ),
    (
        "resume_absent",
        r#"return coroutine.resume()"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #1 to 'resume' (thread expected)"#,
    ),
    (
        "resume_nil",
        r#"return coroutine.resume(nil)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'resume' (thread expected, got nil)"#,
    ),
    (
        "freeze_absent",
        r#"return table.freeze()"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #1 to 'freeze' (table expected)"#,
    ),
    (
        "freeze_nil",
        r#"return table.freeze(nil)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'freeze' (table expected, got nil)"#,
    ),
    (
        "freeze_already_before_protected",
        r#"local mt={}; local t=table.freeze(setmetatable({},mt)); mt.__metatable=false; return table.freeze(t)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'freeze' (table is already frozen)"#,
    ),
    (
        "rawget_zero_results",
        r#"local function none() end; return rawget({},none())"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #2"#,
    ),
    (
        "rawset_zero_results",
        r#"local function none() end; return rawset({},1,none())"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #3"#,
    ),
    (
        "type_zero_results",
        r#"local function none() end; return type(none())"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #1"#,
    ),
    (
        "sub_zero_results",
        r#"local function none() end; return string.sub("a",none())"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #2 to 'sub' (number expected)"#,
    ),
    (
        "len_effects_before_validation",
        r#"local seen=0; local function arg() seen=seen+1; return 1 end; local ok=pcall(function() return string.len({},arg()) end); return tostring(ok)..":"..seen"#,
        r#"true:string:false:1"#,
    ),
    (
        "sub_effects_before_validation",
        r#"local seen=0; local function arg() seen=seen+1; return 1 end; local ok=pcall(function() return string.sub({},arg()) end); return tostring(ok)..":"..seen"#,
        r#"true:string:false:1"#,
    ),
    (
        "insert_effects_before_validation",
        r#"local seen=0; local function arg() seen=seen+1; return 1 end; local ok=pcall(function() return table.insert(false,arg(),arg(),arg()) end); return tostring(ok)..":"..seen"#,
        r#"true:string:false:3"#,
    ),
    (
        "direct_pcall_len",
        r#"local ok,e=pcall(string.len,{}); return tostring(ok)..":"..e"#,
        r#"true:string:false:invalid argument #1 to 'len' (string expected, got table)"#,
    ),
    (
        "direct_pcall_rawget",
        r#"local ok,e=pcall(rawget,{}); return tostring(ok)..":"..e"#,
        r#"true:string:false:missing argument #2"#,
    ),
    (
        "direct_pcall_sub",
        r#"local ok,e=pcall(string.sub,"a"); return tostring(ok)..":"..e"#,
        r#"true:string:false:missing argument #2 to 'sub' (number expected)"#,
    ),
    (
        "direct_pcall_sort",
        r#"local ok,e=pcall(table.sort,{},true); return tostring(ok)..":"..e"#,
        r#"true:string:false:invalid argument #2 to 'sort' (function expected, got boolean)"#,
    ),
    (
        "alias_rawget_missing",
        r#"local alias=rawget; return alias({})"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #2"#,
    ),
    (
        "alias_rawset_missing",
        r#"local alias=rawset; return alias({},1)"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #3"#,
    ),
    (
        "alias_type_missing",
        r#"local alias=type; return alias()"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #1"#,
    ),
    (
        "alias_string_len_type",
        r#"local alias=string.len; return alias({})"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'len' (string expected, got table)"#,
    ),
    (
        "alias_string_sub_missing",
        r#"local alias=string.sub; return alias("a")"#,
        r#"false:string:diagnostic_parity.luau:1: missing argument #2 to 'sub' (number expected)"#,
    ),
    (
        "alias_insert_many",
        r#"local alias=table.insert; return alias({},1,2,3)"#,
        r#"false:string:diagnostic_parity.luau:1: wrong number of arguments to 'insert'"#,
    ),
    (
        "alias_sort_type",
        r#"local alias=table.sort; return alias({},true)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #2 to 'sort' (function expected, got boolean)"#,
    ),
    (
        "alias_next_key",
        r#"local alias=next; return alias({},1)"#,
        r#"false:string:invalid key to 'next'"#,
    ),
    (
        "alias_coroutine_resume_type",
        r#"local alias=coroutine.resume; return alias({})"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'resume' (thread expected, got table)"#,
    ),
    (
        "alias_coroutine_create_type",
        r#"local alias=coroutine.create; return alias({})"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'create' (function expected, got table)"#,
    ),
    (
        "alias_freeze_twice",
        r#"local alias=table.freeze; local t=alias({}); return alias(t)"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'freeze' (table is already frozen)"#,
    ),
    (
        "alias_freeze_locked",
        r#"local alias=table.freeze; return alias(setmetatable({}, {__metatable=false}))"#,
        r#"false:string:diagnostic_parity.luau:1: invalid argument #1 to 'freeze' (table has a protected metatable)"#,
    ),
];

#[test]
fn builtin_diagnostics_match_the_oracle_before_and_after_reset() {
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
        for after_reset in [false, true] {
            if after_reset {
                vm.reset_decision().unwrap();
            }
            for &(name, body, expected) in CASES {
                let source = format!("local ok,value=pcall(function() {body} end); ")
                    + r#"local function text(v) local k=type(v); if k=="table" or k=="function" or k=="thread" then return k end; return tostring(v) end; return tostring(ok)..":"..type(value)..":"..text(value)"#;
                let actual: String = vm
                    .load(&source)
                    .source_name("diagnostic_parity.luau")
                    .eval()
                    .unwrap_or_else(|error| {
                        panic!("{profile:?} {name} after_reset={after_reset}: {error}")
                    });
                assert_eq!(
                    actual, expected,
                    "{profile:?} {name} after_reset={after_reset}"
                );
                assert_eq!(vm.eval::<i32>("return 6*7").unwrap(), 42);
            }
        }
    }
}
