//! verify-conf: run reference-Luau (mlua 0.11 vendored luau709) snippets and print behavior,
//! to validate rustia's 50-conformance table. Each probe prints: LABEL | result-or-ERROR.

use mlua::{Lua, MultiValue, Value};

fn run(lua: &Lua, label: &str, src: &str) {
    // Evaluate as an expression first; if that fails to compile, run as a chunk.
    let chunk = lua.load(src);
    match chunk.eval::<MultiValue>() {
        Ok(mv) => {
            let parts: Vec<String> = mv.iter().map(fmt_val).collect();
            println!("{:<40} => {}", label, if parts.is_empty() { "<no values>".into() } else { parts.join(", ") });
        }
        Err(e) => {
            // collapse to one line
            let msg = format!("{}", e);
            let first = msg.lines().next().unwrap_or("").trim();
            println!("{:<40} => ERROR: {}", label, first);
        }
    }
}

fn fmt_val(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Boolean(b) => format!("{}", b),
        Value::Integer(i) => format!("int({})", i),
        Value::Number(n) => format!("num({})", n),
        Value::String(s) => format!("{:?}", s.to_string_lossy()),
        Value::Table(_) => "<table>".into(),
        Value::Function(_) => "<function>".into(),
        other => format!("{:?}", other),
    }
}

fn main() {
    let lua = Lua::new();

    println!("# ===== oracle: mlua 0.11.6 / luau0-src 0.18.3+luau709 (Luau 0.709) =====\n");

    println!("## (1) table.maxn / table.foreach / table.foreachi");
    run(&lua, "type(table.maxn)", "return type(table.maxn)");
    run(&lua, "type(table.foreach)", "return type(table.foreach)");
    run(&lua, "type(table.foreachi)", "return type(table.foreachi)");
    run(&lua, "table.maxn{[1]=1,[5]=2,[10]=3}", "return table.maxn({[1]=1,[5]=2,[10]=3})");
    run(&lua, "table.foreach sums", "local s=0; table.foreach({10,20,30}, function(k,v) s=s+v end); return s");
    run(&lua, "table.foreachi sums", "local s=0; table.foreachi({10,20,30}, function(i,v) s=s+v end); return s");

    println!("\n## (2) math.map");
    run(&lua, "type(math.map)", "return type(math.map)");
    run(&lua, "math.map(5,0,10,0,100)", "return math.map(5,0,10,0,100)");
    run(&lua, "math.map(0.5,0,1,-1,1)", "return math.map(0.5,0,1,-1,1)");
    run(&lua, "type(math.lerp)", "return type(math.lerp)");
    run(&lua, "math.lerp(0,10,0.25)", "return math.lerp and math.lerp(0,10,0.25)");
    run(&lua, "type(math.clamp)", "return type(math.clamp)");
    run(&lua, "type(math.round)", "return type(math.round)");
    run(&lua, "type(math.sign)", "return type(math.sign)");

    println!("\n## (3) pairs vs __iter");
    run(&lua, "type(pairs)", "return type(pairs)");
    // pairs should IGNORE __iter: define a table whose __iter would error; pairs should still iterate raw.
    run(&lua, "pairs ignores __iter (count)", r#"
        local t = setmetatable({10,20,30}, {__iter=function() error("iter called") end})
        local c=0; for k,v in pairs(t) do c=c+1 end; return c
    "#);
    // bare for-in over table with __iter should USE __iter
    run(&lua, "for-in uses __iter", r#"
        local t = setmetatable({}, {__iter=function(self)
            local i=0; return function() i=i+1; if i<=3 then return i, i*100 end end
        end})
        local s=0; for k,v in t do s=s+v end; return s
    "#);
    // bare for-in over plain table (no __iter) -> builtin iteration
    run(&lua, "for-in plain table builtin", r#"
        local t={5,6,7}; local s=0; for k,v in t do s=s+v end; return s
    "#);
    // __pairs should NOT exist / not be honored
    run(&lua, "__pairs honored? (expect raw=3)", r#"
        local t = setmetatable({1,2,3}, {__pairs=function() error("pairs mm") end})
        local c=0; for k,v in pairs(t) do c=c+1 end; return c
    "#);

    println!("\n## (4) getfenv / setfenv");
    run(&lua, "type(getfenv)", "return type(getfenv)");
    run(&lua, "type(setfenv)", "return type(setfenv)");
    run(&lua, "getfenv() is table", "return type(getfenv())");
    run(&lua, "getfenv(0) is table", "return type(getfenv(0))");

    println!("\n## SPOT-CHECKS");

    println!("\n# S1: # on holey tables (border is any valid border)");
    run(&lua, "#{1,2,nil,4}", "return #({1,2,nil,4})");
    run(&lua, "#{[1]=1,[2]=2,[4]=4}", "return #({[1]=1,[2]=2,[4]=4})");
    run(&lua, "#{nil,nil,3}", "return #({nil,nil,3})");
    run(&lua, "t={1,2,3}; t[2]=nil; #t", "local t={1,2,3}; t[2]=nil; return #t");

    println!("\n# S2: -0.0 table keys / equality");
    run(&lua, "t[-0.0]=1; t[0.0]", "local t={}; t[-0.0]=1; return t[0.0]");
    run(&lua, "-0.0 == 0.0", "return -0.0 == 0.0");
    run(&lua, "tostring(-0.0)", "return tostring(-0.0)");
    run(&lua, "1/(-0.0)", "return 1/(-0.0)");

    println!("\n# S3: tostring of whole numbers (Luau is f64-only)");
    run(&lua, "tostring(1)", "return tostring(1)");
    run(&lua, "tostring(1.0)", "return tostring(1.0)");
    run(&lua, "tostring(3.5)", "return tostring(3.5)");
    run(&lua, "tostring(10/2)", "return tostring(10/2)");
    run(&lua, "tostring(2^53)", "return tostring(2^53)");
    run(&lua, "tostring(0.1)", "return tostring(0.1)");
    run(&lua, "tostring(0.1+0.2)", "return tostring(0.1+0.2)");
    run(&lua, "tostring(1e20)", "return tostring(1e20)");
    run(&lua, "tostring(123456789012345)", "return tostring(123456789012345)");
    run(&lua, "tostring(1/0)", "return tostring(1/0)");
    run(&lua, "tostring(-1/0)", "return tostring(-1/0)");
    run(&lua, "tostring(0/0)", "return tostring(0/0)");
    run(&lua, "'..'..1 (concat fmt)", "return ''..1 .. '|' .. (10/2)");

    println!("\n# S4: math.type / integer subtype presence");
    run(&lua, "type(math.type)", "return type(math.type)");
    run(&lua, "type(1)", "return type(1)");
    run(&lua, "math.maxinteger?", "return tostring(math.maxinteger)");

    println!("\n# S5: integer-division // and modulo signs");
    run(&lua, "7 // 2", "return 7 // 2");
    run(&lua, "-7 // 2", "return -7 // 2");
    run(&lua, "7 // -2", "return 7 // -2");
    run(&lua, "-7 // -2", "return -7 // -2");
    run(&lua, "5.5 // 2", "return 5.5 // 2");
    run(&lua, "-7 % 3", "return -7 % 3");
    run(&lua, "7 % -3", "return 7 % -3");
    run(&lua, "5.5 % 2", "return 5.5 % 2");
    run(&lua, "5 % 0 (int)", "return 5 % 0");
    run(&lua, "5 // 0", "return 5 // 0");

    println!("\n# S6: error message text");
    run(&lua, "index nil field", "local x=nil; return x.y");
    run(&lua, "call nil", "local x=nil; return x()");
    run(&lua, "arith on string", "return {} + 1");
    run(&lua, "concat table", "return {} .. 'a'");
    run(&lua, "compare num<string", "return 1 < 'a'");
    run(&lua, "length of number", "return #5");

    println!("\n# S7: string.format edge cases");
    run(&lua, "format %d of 3.0", "return string.format('%d', 3.0)");
    run(&lua, "format %d of 3.5", "return string.format('%d', 3.5)");
    run(&lua, "format %d of '5'", "return string.format('%d', '5')");
    run(&lua, "format %s of number", "return string.format('%s', 42)");
    run(&lua, "format %q", "return string.format('%q', 'a\"b\\nc')");
    run(&lua, "format %x neg", "return string.format('%x', -1)");
    run(&lua, "format %5.2f", "return string.format('%5.2f', 3.14159)");
    run(&lua, "format %g of 1.0", "return string.format('%g', 1.0)");
    run(&lua, "format %s of table(no mt)", "return string.format('%s', {})");
    run(&lua, "format %s of true", "return string.format('%s', true)");

    println!("\n# S8: coroutine.close / select");
    run(&lua, "type(coroutine.close)", "return type(coroutine.close)");
    run(&lua, "close dead coro", "local c=coroutine.create(function() end); coroutine.resume(c); return coroutine.close(c)");
    run(&lua, "close suspended", "local c=coroutine.create(function() coroutine.yield() end); coroutine.resume(c); return coroutine.close(c)");
    run(&lua, "close running (self)", "local c; c=coroutine.wrap(function() return coroutine.close(c) end); return pcall(c)");
    run(&lua, "select('#', a,b,c)", "return select('#', 1,2,3)");
    run(&lua, "select('#')", "return select('#')");
    run(&lua, "select(2, 'a','b','c')", "return select(2,'a','b','c')");
    run(&lua, "select(-1, 'a','b','c')", "return select(-1,'a','b','c')");
    run(&lua, "select(0,...)", "return select(0, 'a')");

    println!("\n# S9: typeof / type / vector / bit32");
    run(&lua, "type(typeof)", "return type(typeof)");
    run(&lua, "typeof(1)", "return typeof(1)");
    run(&lua, "type(vector)", "return type(vector)");
    run(&lua, "typeof(vector.create(1,2,3))", "return type(vector) ~= 'nil' and typeof(vector.create(1,2,3)) or 'no-vector'");
    run(&lua, "type(bit32)", "return type(bit32)");
    run(&lua, "bit32.band(6,3)", "return bit32.band(6,3)");
    run(&lua, "type(utf8)", "return type(utf8)");
    run(&lua, "type(os.time)", "return type(os.time)");
    run(&lua, "type(os.clock)", "return type(os.clock)");
    run(&lua, "type(loadstring)", "return type(loadstring)");
    run(&lua, "type(load)", "return type(load)");
    run(&lua, "type(newproxy)", "return type(newproxy)");
    run(&lua, "type(gcinfo)", "return type(gcinfo)");
    run(&lua, "type(collectgarbage)", "return type(collectgarbage)");
    run(&lua, "type(string.pack)", "return type(string.pack)");
    run(&lua, "type(string.split)", "return type(string.split)");
    run(&lua, "type(rawlen)", "return type(rawlen)");

    println!("\n# S10: numeric for-loop & misc edge");
    run(&lua, "0.1 step for count", "local c=0; for i=1,2,0.1 do c=c+1 end; return c");
    run(&lua, "next on empty", "return next({})");
    run(&lua, "tostring(nil)/(true)", "return tostring(nil)..'|'..tostring(true)");
    run(&lua, "string.rep big? len", "return #string.rep('ab',3)");
    run(&lua, "('%d'):format? methods", "return ('hello'):upper()");
    run(&lua, "tonumber('0x1A')", "return tonumber('0x1A')");
    run(&lua, "tonumber('  10  ')", "return tonumber('  10  ')");
    run(&lua, "tonumber('1e3')", "return tonumber('1e3')");
    run(&lua, "tonumber('inf')", "return tostring(tonumber('inf'))");
    run(&lua, "math.floor(-0.5)", "return math.floor(-0.5)");
    run(&lua, "3 == 3.0", "return 3 == 3.0");
    run(&lua, "tostring(1) key vs '1' key", "local t={}; t[1]=10; return t['1']");
}
