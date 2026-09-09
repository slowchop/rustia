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
    match byte(0) % 8 {
        0 => body.push_str("local function unused() local x = end"),
        1 => body.push_str(failure),
        2 => body.push_str("local t={}; local ok,e=pcall(error,t); return tostring(ok)..':'..tostring(rawequal(t,e))"),
        3 => body.push_str(&format!("local ok,e=pcall(function() {failure} end); return tostring(ok)..':'..type(e)")),
        4 => body.push_str(&format!("local ok,e=xpcall(function() {failure} end,function(e) return type(e) end); return tostring(ok)..':'..e")),
        5 => body.push_str(&format!("local c=coroutine.create(function() coroutine.yield('pause'); {failure} end); local a,b=coroutine.resume(c); local d,e=coroutine.resume(c); return tostring(a)..':'..b..':'..tostring(d)..':'..type(e)")),
        6 => body.push_str(&format!("local f; pcall(function() local x={}; f=function() x=x+1; return x end; {failure} end); return tostring(f())..':'..tostring(f())", byte(3))),
        _ => body.push_str(&format!("local ok,e=xpcall(function() {failure} end,function() error('handler',0) end); return tostring(ok)..':'..e")),
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
