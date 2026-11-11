//! Generated, bounded language programs; engine-specific quotas are not oracle semantics.
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Shared with CI smoke so new generated shapes are not omitted from its range.
pub const SOURCE_SHAPE_COUNT: u8 = 52;

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
    match byte(0) % SOURCE_SHAPE_COUNT {
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
        28 => body.push_str("local function pass(...) return ... end; local function many() return 4,5 end; local t={select('#',pass(pass(pass(pass(many())))))}; return tostring(t[1])"),
        29 => body.push_str("local a,b; a=function(n) if n==0 then return 4,5 end; return string.len('abc',b(n-1)) end; b=function(n) if n==0 then return 4,5 end; return string.len('abc',a(n-1)) end; string.len=function(...) return select('#',...),7 end; return tostring(a(20))"),
        30 => body.push_str("local a,b={},{}; setmetatable(a,{__call=function(self,n) if n==0 then return 4,5 end; return string.len('abc',b(n-1)) end}); setmetatable(b,{__call=function(self,n) if n==0 then return 4,5 end; return string.len('abc',a(n-1)) end}); string.len=function(...) return select('#',...),7 end; return tostring(a(20))"),
        31 => body.push_str("local function argument() observed=observed+1; return 'abc' end; local function run(n) local unused=string.len(argument()); if n==0 then return 0 end; return 1+run(n-1) end; return tostring(run(20))"),
        32 => body.push_str("local left,right={},{}; setmetatable(left,{__call=function(self,n) if n==0 then return 0 end; return 1+right(n-1) end}); setmetatable(right,{__call=function(self,n) if n==0 then return 0 end; return 1+left(n-1) end}); return tostring(left(40))"),
        33 => body.push_str("local receiver={}; setmetatable(receiver,{__call=function(self,n) assert(self==receiver); return n+coroutine.yield('wait') end}); local owner=coroutine.create(function() return receiver(20) end); local a,b=coroutine.resume(owner); local c,d=coroutine.resume(owner,22); return tostring(a)..':'..b..':'..tostring(c)..':'..d"),
        34 => body.push_str("local inner=setmetatable({},{__call=function() return 42 end}); local outer=setmetatable({},{__call=inner}); local function args() return 1,2 end; local a,b=pcall(function() return outer() end); local c,d=pcall(function() return outer(args()) end); return tostring(a)..':'..type(b)..':'..tostring(c)..':'..type(d)"),
        35 => body.push_str("local t={m=function() return 'old' end}; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 1,2 end; return t:m(arg())"),
        36 => body.push_str("local t=setmetatable({}, {__index=function() coroutine.yield('lookup') end}); local c=coroutine.create(function() return t:missing() end); local a,b=coroutine.resume(c); local d,e=coroutine.resume(c); return tostring(a)..':'..b..':'..tostring(d)..':'..e"),
        37 => body.push_str("local t={m=function() return 'old' end}; local function arg() coroutine.yield('argument'); t.m=function() coroutine.yield('method'); return 'new' end; return 1,2 end; local c=coroutine.create(function() return t:m(arg()) end); local a,b=coroutine.resume(c); local d,e=coroutine.resume(c); local f,g=coroutine.resume(c); return tostring(a)..':'..b..':'..tostring(d)..':'..e..':'..tostring(f)..':'..g"),
        38 => body.push_str("local t={m=function() return 'old' end}; local function arg() t={m=function() return 'new' end}; return 1,2 end; return t:m(arg())"),
        39 => body.push_str("local t={m=function(self,...) return select('#',...) end}; local function run(...) return ({t:m(...)})[1] end; return tostring(run(1,2))"),
        40 => {
            let ty = ["number", "(number)", "((number))", "(number)?", "number | string", "number & unknown", "() -> (number,string)", "Foo<(number,string)>"][usize::from(byte(2) % 8)];
            body.push_str(&format!("local value = 1 :: {ty}; return tostring(value)"));
        }
        41 => {
            let ty = ["()", "(number,string)", "((number,string))", "(...number)", "number...", "...number", "(number,string)?", "(number,string) | number"][usize::from(byte(2) % 8)];
            body.push_str(&format!("local value = 1 :: {ty}; return 'invalid assertion'"));
        }
        42 => {
            let params = ["T=number", "T, U=string", "T=number, U=string", "T=(number)", "T=() -> (number,string)", "T=Foo<(number,string)>", "T=typeof(observed)", "T=number, P...=()"][usize::from(byte(2) % 8)];
            body.push_str(&format!("type Alias<{params}> = {{value:T}}; return 'default'"));
        }
        43 => {
            let params = ["T=", "T=number,U", "T=number,P...", "P...,T", "T=number,", "T=(number,string)", "T=...number", "T=()"][usize::from(byte(2) % 8)];
            body.push_str(&format!("type Alias<{params}> = T; return 'invalid default'"));
        }
        44 => {
            let default = ["()", "(number)", "(number,string)", "...number", "U...", "(number,...string)", "(number,U...)", "...number?"][usize::from(byte(2) % 8)];
            body.push_str(&format!("export type Alias<P...={default}> = number; return 'pack default'"));
        }
        45 => {
            let default = ["number", "() -> number", "(number) | string", "((number,string))", "(U...,number)", "(...number,string)", "U...?", "(number,)"][usize::from(byte(2) % 8)];
            body.push_str(&format!("type Alias<P...={default}> = number; return 'invalid pack default'"));
        }
        46 => body.push_str("type Alias<T=number> = T; local function many() return 4,5 end; local fn = many :: () -> (number,number); return tostring(select('#',fn()))..':'..tostring(select('#',fn() :: Alias))"),
        47 => body.push_str("local function f<T=number>() return 1 end; return 'invalid function default'"),
        48 => {
            let ty = ["<T>(T)->T", "<T>(value:T)->T", "<T>(T,value:T)->T", "<T>(T)-><U>(U)->T", "<type>(type:type)->type", "<T,T>(T)->T", "(<T>(T)->T)?", "<T>() -> (number,string)?"][usize::from(byte(2) % 8)];
            body.push_str(&format!("type Identity = {ty}; local function id(v) return v end; local typed=id :: Identity; return tostring(typed(42))"));
        }
        49 => {
            let ty = ["<T>(T)", "<T>()", "<>(T)->T", "<T=number>(T)->T", "<P...=()>() -> ()", "<P...,T>(P...)->T", "<T,>(T)->T", "<T>number"][usize::from(byte(2) % 8)];
            body.push_str(&format!("type Invalid = {ty}; return 'invalid generic function'"));
        }
        50 => {
            let ty = ["<P...>(P...)->P...", "<P...>(...number)->P...", "<T,P...>(T,P...)->P...", "<T>(x:T,...T)->T", "<T>(T)->() | nil", "<P...,P...>(P...)->P...", "<T>(T)->(x:T)->T", "<T>(x:T,string)->T"][usize::from(byte(2) % 8)];
            body.push_str(&format!("type Box<F = {ty}> = F; local function many(...) return ... end; local packed=many :: (<P...>(P...)->P...); return tostring(select('#',packed(1,2)))"));
        }
        _ => {
            let ty = ["<T>(x:T...)->T", "<T>(x:...T)->T", "<T>(T...,number)->T", "<T>(...T,number)->T", "<T>((T...))->T", "<T>(())->T", "<T>((number,string))->T", "<T>(T)->(value:T)"][usize::from(byte(2) % 8)];
            body.push_str(&format!("type Invalid = {ty}; return 'invalid function parameter pack'"));
        }
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
