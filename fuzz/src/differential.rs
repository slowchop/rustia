//! Generated, bounded language programs; engine-specific quotas are not oracle semantics.
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn source_from_bytes(data: &[u8]) -> String {
    let byte = |index: usize| data.get(index).copied().unwrap_or(0);
    let n = byte(1);
    let mut body = format!("observed = {n};\n");
    for value in data.iter().skip(4).take(8) {
        body.push_str(&format!("observed = observed + {};\n", value % 4));
    }
    let failure = match byte(2) % 5 {
        0 => "error('deliberate', 0)",
        1 => "local x=nil; return x.field",
        2 => "local t={}; t[0/0]=1",
        3 => "local x=false; return x()",
        _ => "local x={}; return x + 1",
    };
    let invalid_builtin = match byte(2) % 7 {
        0 => "tostring()",
        1 => "tonumber()",
        2 => "rawget({})",
        3 => "table.insert()",
        4 => "math.unreal()",
        5 => "table.unreal()",
        _ => "string.unreal()",
    };
    let extra_argument_call = match byte(2) % 6 {
        0 => "type(1, 99)",
        1 => "string.len('abc', 99)",
        2 => "string.upper('abc', 99)",
        3 => "table.getn({4}, 99)",
        4 => "next({a=1}, nil, 99)",
        _ => "tonumber('1', 10, 99)",
    };
    match byte(0) % 29 {
        0 => body.push_str("local function unused() local x = end"),
        1 => body.push_str(failure),
        2 => body.push_str("local t={}; local ok,e=pcall(error,t); return tostring(ok)..':'..tostring(rawequal(t,e))"),
        3 => body.push_str(&format!("local ok,e=pcall(function() {failure} end); return tostring(ok)..':'..type(e)")),
        4 => body.push_str(&format!("local ok,e=xpcall(function() {failure} end,function(e) return type(e) end); return tostring(ok)..':'..e")),
        5 => body.push_str(&format!("local c=coroutine.create(function() coroutine.yield('pause'); {failure} end); local a,b=coroutine.resume(c); local d,e=coroutine.resume(c); return tostring(a)..':'..b..':'..tostring(d)..':'..type(e)")),
        6 => body.push_str(&format!("local f; pcall(function() local x={}; f=function() x=x+1; return x end; {failure} end); return tostring(f())..':'..tostring(f())", byte(3))),
        7 => body.push_str(&format!("local ok,e=xpcall(function() {failure} end,function() error('handler',0) end); return tostring(ok)..':'..e")),
        8 => body.push_str(&format!("return '' .. {invalid_builtin}")),
        9 => body.push_str(&format!("if false then local value = '' .. {invalid_builtin} end; return 'ok'")),
        10 => body.push_str(&format!("return {invalid_builtin}")),
        11 => body.push_str(&format!("return '' .. {extra_argument_call}")),
        12 => body.push_str("local function extra(n) observed=observed*10+n; return n end; return type(extra(1),extra(2),extra(3))..':'..observed"),
        13 => {
            let iterator = if byte(2) % 2 == 0 { "ipairs" } else { "pairs" };
            body.push_str(&format!("local function extra() observed=observed+1; return 99 end; local out=''; for k,v in {iterator}({{4}},extra()) do out=out..k..':'..v end; return out..':'..observed"));
        }
        14 => body.push_str("local function unused() break end; return 'ok'"),
        15 => body.push_str(&format!("local function unused() return '' .. {invalid_builtin} end; return 'ok'")),
        16 => body.push_str("local t={}; local function many() return 4,99 end; table.insert(t,1,many()); return 'wrong'"),
        17 => body.push_str("local function empty() end; return 'x'..type(empty())"),
        18 => body.push_str("local function empty() end; return 'x'..type((empty()))"),
        19 => body.push_str("local function extra() string.len=function() return 100 end; return 99 end; return ''..string.len('abc',extra())"),
        20 => body.push_str("local function extra() string.len=function() return 100 end; return 99 end; return ''..(string.len)('abc',extra())"),
        21 => body.push_str("local len=string.len; local function extra() string.len=function() return 100 end; return 99 end; return ''..len('abc',extra())"),
        22 => body.push_str("local function argument() coroutine.yield('argument'); string.len=function(...) coroutine.yield('callee'); return select('#',...) end; return 99,88 end; local thread=coroutine.create(function() return 0+string.len('abc',argument()) end); local a,b=coroutine.resume(thread); local c,d=coroutine.resume(thread); local e,f=coroutine.resume(thread); return tostring(a)..':'..tostring(b)..':'..tostring(c)..':'..tostring(d)..':'..tostring(e)..':'..tostring(f)"),
        23 => body.push_str("local object={pass=function(self,...) return ... end}; local function argument() string.len=function(...) return select('#',...) end; return 99,88 end; return ''..object:pass(string.len('abc',argument()))"),
        24 => body.push_str("local function empty() end; for k,v in ipairs(empty()) do end; return 'wrong'"),
        25 => body.push_str("local function pass(...) return ... end; local function empty() end; local t={type(pass(pass(pass(empty()))))}; return tostring(t[1])"),
        26 => body.push_str("local function pass(...) return ... end; local function run(...) return 0+select('#',pass(pass(pass(...)))) end; return tostring(run(4,5))"),
        27 => body.push_str("local function run(n) if n==0 then return 4,5 end; return string.len('abc',run(n-1)) end; string.len=function(...) return select('#',...),7 end; return tostring(run(20))"),
        _ => body.push_str("local function pass(...) return ... end; local function many() return 4,5 end; local t={select('#',pass(pass(pass(pass(many())))))}; return tostring(t[1])"),
    }
    body
}

/// Runs inside the instrumented fuzz process so VM branches guide mutations.
pub fn observe_rustia(source: &[u8]) -> String {
    let mut vm = rustia::RustiaBuilder::standard()
        .profile(rustia::Profile::LuauCompat)
        .fuel(100_000)
        .memory_limit_bytes(8 * 1024 * 1024)
        .build()
        .unwrap();
    vm.insert_global("observed", 0).unwrap();
    let result = vm
        .load(source)
        .source_name("differential.luau")
        .eval::<String>();
    let observed: i32 = vm.eval("return observed").unwrap();
    match result {
        Ok(value) => format!("ok\n{observed}\n{value}\n"),
        Err(error) => match error.kind() {
            rustia::ErrorKind::Source(_) | rustia::ErrorKind::Compile(_) => {
                format!("frontend\n{observed}\n")
            }
            rustia::ErrorKind::Vm(_) => format!("runtime\n{observed}\n"),
            _ => panic!("unexpected boundary: {error:?}"),
        },
    }
}

pub fn fuzz_differential(data: &[u8]) -> bool {
    if data.len() > 64 {
        return false;
    }
    let source = source_from_bytes(data);
    let actual = observe_rustia(source.as_bytes());
    let worker = std::env::var_os("RUSTIA_DIFF_WORKER").expect("set the pinned oracle worker path");
    let mut child = Command::new(worker)
        .arg("luau")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start oracle worker");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("oracle timeout requires triage; input {data:?}");
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(
        status.success(),
        "oracle worker failed: {status}; input {data:?}"
    );
    let mut expected = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .take(65537)
        .read_to_string(&mut expected)
        .unwrap();
    assert!(expected.len() <= 65536, "oracle output exceeds bound");
    assert_eq!(actual, expected, "input {data:?}\n{source}");
    true
}
