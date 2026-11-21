//! Guest-observable results of optimization examples in pinned Luau 0.709
//! [Compiler.test.cpp](https://github.com/luau-lang/luau/blob/b968ef742741bb2b703afc3b3c53f06608c87481/tests/Compiler.test.cpp#L5739-L10373).

use crate::agrees;

#[test]
fn loop_steps_and_dependent_bounds() {
    agrees(
        r##"
local function walks(first, last, step)
    local seen = {}
    for i = first, last, step do table.insert(seen, i) end
    return table.concat(seen, ",")
end
local hits = {}
for i = 0, 1 do
    for j = 0, i do table.insert(hits, i * 2 + j + 1) end
end
assert(walks(1, 2, 1) == "1,2" and walks(2, 1, -1) == "2,1")
assert(walks(1, 4, 2) == "1,3" and walks(2, 1, 1) == "")
return table.concat(hits, ","), walks(0, 1, 0.5), walks(4294967295, 4294967296, 1)
"##,
    );
}

#[test]
fn loop_exits_and_captures() {
    agrees(
        r##"
local values, log = {}, ""
for i = 1, 5 do
    local j = i
    values[i] = function() return j end
    if i == 2 then continue end
    if i == 4 then break end
    j += 10
    for k = 1, 3 do
        if k == 1 then continue end
        if k == 3 then break end
        log ..= tostring(i) .. tostring(k)
    end
end
assert(log == "1232" and values[5] == nil)
return values[1](), values[2](), values[3](), values[4]()
"##,
    );
}

#[test]
fn loop_bodies_and_bitwise_calls() {
    agrees(
        r##"
local function sums(n)
    local squares, total = {}, 0
    for i = 1, n do squares[i] = i * i * i; total += i end
    return squares[n], total
end
local function cipher(nonce, op)
    local block = {}
    for i = 0, 3 do block[i + 1] = op(bit32.rshift(nonce, i * 8), 0xff, 0xff) end
    return table.concat(block, ",")
end
local cube, sum = sums(3)
assert(cube == 27 and sum == 6)
return cipher(0x12345678, bit32.band), cipher(0x12345678, function(a, b, c) return b - c end), sums(100)
"##,
    );
}

#[test]
fn calls_keep_argument_order_and_result_adjustment() {
    agrees(
        r##"
local log = ""
local function mark(s) log ..= s; return s end
local function choose(test, a, b) return if test then a else b end
local function one(a) return a end
local function plus(a, b) return a + b end
local function forward(...) return plus(...) end
assert(choose(false, mark("a"), mark("b")) == "b")
assert(one(mark("c"), mark("d")) == "c")
local function empty() end
local function sometimes(t) if t then return 42 end end
local a, b = sometimes(false)
assert(a == nil and b == nil and select("#", empty()) == 0 and select("#", sometimes(true)) == 1)
return log, one(), plus(math.modf(1.5)), forward(7, 8)
"##,
    );
}

#[test]
fn call_chains_and_nested_arguments() {
    agrees(
        r##"
local function add(a,b) return a + b end
local function chain(x) return add(x, 1) * add(x, -1) end
local function baz() return (chain(42)) end
local function twice(x,y) return add(add(x,1), add(2,y)) end
local x,y = 5,9
assert(baz() == 1763)
return add(x,1), add(1,x), add(1,2), add(x,y), twice(x,y), add(x,add(y,3))
"##,
    );
}

#[test]
fn recursive_calls_and_immediate_closures() {
    agrees(
        r##"
local function fact(n) return if n <= 1 then 1 else fact(n-1)*n end
local function checked(n) assert(n >= 1); return fact(n) end
local function by_args(a,b,c)
    return ((function(t,x,y) return if t then x else y end)(a,b,c))
end
local function by_capture(a,b,c)
    return ((function() return if a then b else c end)())
end
local function adder(x) return function(y) return x + y end end
return checked(5), by_args(false, 4, 7), by_capture(true, 4, 7), adder(1)(2)
"##,
    );
}

#[test]
fn captured_arguments_and_mutation() {
    agrees(
        r##"
local x = 8
local function capture(a) a = a or 42; return function() return a end end
local function from_outer() return x end
local saved, default = capture(x), capture(nil)
local function mutate_outer(a) x = 26; return a end
local function mutate_argument(a) a = 42; return a end
local original = mutate_outer(x :: number)
local after = mutate_argument(x :: number)
local function saved_local(a,b)
    local c,d = (a :: number), b
    a += 1; b += 2
    return function() return c+d end
end
assert(saved() == 8 and original == 8 and from_outer() == 26)
return x, after, default(), saved_local(10,20)()
"##,
    );
}

#[test]
fn calls_forward_primitive_results() {
    agrees(
        r##"
local function id(a) return a end
local function wrap(f, ...) return f(...) end
local function triple(a,b) return a,nil,b end
local function swapped(x,y) return y,x end
local function multiple(x,y) return (x), y :: number end
local s,t = swapped(10,20)
local p,q = multiple(3,4)
local function from_vararg(...) return 42 end
assert(from_vararg(1,nil,3) == 42)
assert(id(nil) == nil and select("#", wrap(triple, 1, 2)) == 3)
return s,t,p,q,wrap(triple, 10, 20)
"##,
    );
}

#[test]
fn callbacks_and_mutable_comparators() {
    agrees(
        r##"
local calls = 0
local function test(a,b,c,comp)
    return comp(a,b) and comp(b,c)
end
local function greater(a,b) calls += 1; return a > b end
local function check(x,y,z) return test(x,y,z,greater) end
assert(check(3,2,1) and not check(1,2,3) and calls == 3)
greater = function(a,b) return a < b end
local function apply(a,b,c,d,op) return op(a,b) * op(c,d) end
return check(1,2,3), test(2,2,2,function(a,b) return a >= b end),
    apply(5,7,2,4,math.max), apply(5,7,2,4,math.min)
"##,
    );
}

#[test]
fn eliminated_branches_keep_side_effects() {
    agrees(
        r##"
local enabled = false
local log = ""
local function avoided(n) if enabled then error("unused branch") end; return n + 42 end
local function score(kind)
    local sum = 0
    for i=1,5 do
        if kind == 1 then sum += i elseif kind == 2 then sum -= i else log ..= "x" end
    end
    return sum
end
local function lookup() log ..= "r"; return 17 end
local obj = setmetatable({}, {__index=lookup})
local unused = 5, obj.missing
local function read() return obj.missing end
read()
if false and obj.missing then log ..= "!" end
if obj.missing and false then log ..= "!" end
assert(log == "rrr")
local function field(t) return if t.x then t.x else 0 end
return avoided(1), score(1), score(2), field(obj), log
"##,
    );
}

#[test]
fn early_returns_stay_inside_the_callee() {
    agrees(
        r##"
local source = {present = {value = 7}}
local props, log = {}, ""
local function perform(name, update)
    local obj = source[name]
    if obj then props[name] = obj.value else return end
    if update then update(props[name]) end
end
local function all()
    perform("absent", function() error("unreachable callback") end)
    perform("present", function(n) log ..= tostring(n) end)
    log ..= "!"
end
all()
assert(log == "7!" and props.absent == nil)
return log, props.present
"##,
    );
}

#[test]
fn builtin_errors_and_constant_operands() {
    agrees(
        r##"
local function set(key) return rawset({}, key) end
local function result(f) return (pcall(f)) end
local function index(key) return {[""] = 17, key = key} end
local k = "key"
assert(index(8)[""] == 17 and index(8)[k] == 8)
assert(not result(function() set(false) end) and not result(function() set({}) end))
return result(function() math.abs() end), result(function() math.max(1,true) end),
    result(function() math.min(1,true) end), result(function() bit32.bor(1,true) end),
    result(function() bit32.band(1,true) end), result(function() bit32.bxor(1,true) end),
    result(function() bit32.btest(1,true) end), bit32.rshift(10,42),
    bit32.extract(1,2,"3"), select("#",string.byte("abc",42)), typeof(vector.create(1,0,0))
"##,
    );
}

#[test]
fn builtin_calls_and_result_arity() {
    agrees(
        r##"
local function noargs() end
local function many() return -4,7,9 end
local function type_results(x)
    local y,z = type(x)
    assert(z == nil)
    return type(y,z)
end
local MT = {}
local function new() return setmetatable({}, MT) end
assert(getmetatable(new()) == MT)
local function lanes(root)
    local mask = 0x40000000
    local values = bit32.band(root.pendingLanes, bit32.bnot(mask))
    if values ~= 0 then return values end
    if bit32.band(values, mask) ~= 0 then return mask end
    return 0
end
assert(select("#", math.abs(-42)) == 1)
return math.abs(many()), math.max(0,many()), bit32.extract(14,1,noargs()),
    (pcall(function() return type(noargs()) end)), type_results(4), lanes({pendingLanes=0x40000008}), lanes({pendingLanes=0x40000000})
"##,
    );
}

#[test]
fn simultaneous_assignment_and_discarded_results() {
    agrees(
        r##"
local log = ""
local function f(i) log ..= tostring(i); return i end
local a,b,c = f(1),f(2),f(3)
a,b,c = c,a,b
assert(a == 3 and b == 1 and c == 2)
a,b,c = b,c,a
assert(a == 1 and b == 2 and c == 3)
local t = {10,20}
t[1],t[2] = t[2],t[1]
local function more() log ..= "m"; return 8,9 end
local d
a,b,c,d = 1,more()
assert(a == 1 and b == 8 and c == 9 and d == nil)
local indices = {}
local k = 2
k,indices[k] = 1,42
a,b = 4,5,f(4)
local first, last = 3, 4
local left, right = {}, {}
first, left[first], right[last], last = 1, more()
assert(first == 1 and left[3] == 8 and right[4] == 9 and last == nil)
local alias = (((a)))
alias = alias :: number
return a,b,log,k,indices[1],indices[2],t[1],t[2],alias
"##,
    );
}

#[test]
fn loop_break_targets_and_value_conditions() {
    agrees(
        r##"
local function run(outer, branch)
    local visits = 0
    repeat
        if branch then
            if outer then break end
            while true do break end
        elseif outer then
            break
        end
        visits += 1
    until visits == 2
    return visits
end
local function choose(x,y)
    return (if x then x :: number else y), (if x then y else x :: number)
end
local a,b = choose(false,3)
local c,d = choose(0,3)
assert(a == 3 and b == false and c == 0 and d == 3)
return run(true,true), run(true,false), run(false,true), run(false,false)
"##,
    );
}

#[test]
fn environment_sensitive_math_and_vararg_calls() {
    agrees(
        r##"
local function foo(...) return 42 end
local function read() return math.pi * 2, math.sin(0) end
local pi2,sine = read()
local env = getfenv(read)
local old = env.math
env.math = {pi = 4, sin = function(n) return n + 8 end}
local changed, value = read()
env.math = old
local restored, restored_sine = read()
assert(changed == 8 and value == 8 and restored == pi2 and restored_sine == sine)
return foo(1,2,3), pi2, sine, changed, value
"##,
    );
}

#[test]
fn reverse_constants_and_byte_strings() {
    agrees(
        r##"
local function reverse(x: number)
    return 2+x, 2-x, 2*x, 2/x, 2%x, 2//x, 2^x
end
local hello,world = "hello","world"
assert(hello .. " " .. world == "hello world")
return "".."", "a".."", "".."a", string.char(49,50,51,52,53,54),
    string.char(), string.char(0,0,0), string.char(49,50,0,52,53,0), reverse(4)
"##,
    );
}

#[test]
fn constant_builtin_results() {
    for source in [
        "return math.abs(-42), math.acos(1), math.asin(0), math.atan2(0, 1), math.atan(0), math.ceil(1.5), math.cosh(0), math.cos(0)",
        "return math.deg(3.14159265358979323846), math.exp(0), math.floor(-1.5), math.fmod(7, 3), math.ldexp(0.5, 3), math.log10(100), math.log(1), math.log(4, 2)",
        "return math.log(64, 4), math.max(1, 2, 3), math.min(1, 2, 3), math.pow(3, 3), math.floor(math.rad(180)), math.sinh(0), math.sin(0), math.sqrt(9)",
        "return math.tanh(0), math.tan(0), bit32.arshift(-10, 1), bit32.arshift(10, 1), bit32.band(1, 3), bit32.bnot(-2), bit32.bor(1, 2), bit32.bxor(3, 7)",
        "return bit32.btest(1, 3), bit32.extract(100, 1, 3), bit32.lrotate(100, -1), bit32.lshift(100, 1), bit32.replace(100, 5, 1, 3), bit32.rrotate(100, -1), bit32.rshift(100, 1), type(100)",
        "return string.byte(\"a\"), string.byte(\"abc\", 2), string.len(\"abc\"), typeof(true), math.clamp(-1, 0, 1), math.sign(77), math.round(7.6), bit32.extract(-1, 31)",
        "return bit32.replace(100, 1, 0), math.log(100, 10), typeof(nil), type(vector.create(1, 0, 0)), (type(\"fin\")), math.isnan(0/0), math.isnan(0), math.isinf(math.huge)",
        "return math.isinf(-4), math.isfinite(42), math.isfinite(-math.huge)",
    ] {
        agrees(source);
    }
}

#[test]
fn zero_argument_builtin_calls() {
    for name in [
        "math.abs",
        "math.acos",
        "math.asin",
        "math.atan2",
        "math.atan",
        "math.ceil",
        "math.cosh",
        "math.cos",
        "math.deg",
        "math.exp",
        "math.floor",
        "math.fmod",
        "math.ldexp",
        "math.log10",
        "math.log",
        "math.max",
        "math.min",
        "math.pow",
        "math.rad",
        "math.sinh",
        "math.sin",
        "math.sqrt",
        "math.tanh",
        "math.tan",
        "bit32.arshift",
        "bit32.band",
        "bit32.bnot",
        "bit32.bor",
        "bit32.bxor",
        "bit32.btest",
        "bit32.extract",
        "bit32.lrotate",
        "bit32.lshift",
        "bit32.replace",
        "bit32.rrotate",
        "bit32.rshift",
        "type",
        "string.byte",
        "string.len",
        "typeof",
        "math.clamp",
        "math.sign",
        "math.round",
    ] {
        // Keep the call direct so the compiler sees the exact builtin and arity.
        let source = format!("assert(type({name}) == 'function'); local ok, v = pcall(function() return {name}() end); if ok then return true, v end; return false");
        agrees(&source);
    }
}

#[test]
fn optimization_directives_preserve_identity_calls() {
    for directive in ["", "--!optimize 1", "--!optimize 2"] {
        agrees(&format!(
            "{directive}\nlocal function id(a) return a end; return id(42)"
        ));
    }
}

#[test]
fn ordinary_type_annotations_are_erased() {
    agrees(
        r##"
type Number = number
type List = {number}
type Identity<T> = T
local function read(x: Identity<Number>, nums: List, callback: (number) -> number): number
    return callback(nums[1]) + x
end
local function grouped(x: (string | nil), y: nil) return x, y end
local function annotated(a: string & number, b: Instance, c: vector) return a, b, c.X end
local tab = {a=1}
function tab:method(n: number) return self.a + n end
assert(read(3, {5}, function(n) return n * 2 end) == 13 and tab:method(4) == 5)
local x,y = grouped("value", nil)
return x,y,annotated(3, "opaque", vector.create(1,2,3))
"##,
    );
}
