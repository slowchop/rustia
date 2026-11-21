//! Guest-visible adaptations of Luau 0.709 tests/Compiler.test.cpp (MIT), lines 225–2909.

use crate::{agrees, rejects};

#[test]
fn literal_returns_and_function_call_targets() {
    agrees("return 5, 6.5");
    agrees("local function foo(a, b) return b end function test() return foo(2) end return foo(1, 3), test()");
    agrees(
        r#"
        local trace = ""
        local math = {
            foo = function(v) trace ..= "f" return v or 0 end,
            bar = function() trace ..= "b" return 3 end,
        }
        local first = math.foo()
        local nested = math.foo(math.bar())
        local foo = 7
        foo = math.foo(foo)
        return first, nested, foo, trace
    "#,
    );
}

#[test]
fn resolving_local_imports_and_rebound_globals() {
    agrees("return math.max(1, 2)");
    agrees("local math = math return math.max(1, 2)");
    agrees(r#"local math = math or require("math-polyfill") return math.max(1, 2)"#);
    agrees("math = {} function math.max() return 0 end function test() return math.max(1, 2) end return test()");
}

#[test]
fn table_backed_host_objects_and_captured_self() {
    agrees(
        r#"
        local Instance = {new = function(class, parent)
            return {ClassName = class, Parent = parent, GetMass = function(self) return self.Parent.mass end}
        end}
        local workspace = {mass = 4}
        local Vector3 = {new = vector.create}
        local Enum = {EasingStyle = {Linear = "Linear"}}
        local part = Instance.new("Part", workspace)
        part.Size = Vector3.new(1, 2, 3)
        return part.Size.Z * part:GetMass(), part.ClassName, Enum.EasingStyle.Linear
    "#,
    );
    agrees(
        r#"
        local class = {}
        function class:update(text)
            local function updateTooltip() self.text = text end
            updateTooltip()
        end
        local a, b = {}, {}
        class.update(a, "first")
        class.update(b, "second")
        return a.text, b.text
    "#,
    );
}

#[test]
fn concat_associativity_with_observable_grouping() {
    agrees(
        r#"
        local mt = {}
        local function wrap(label) return setmetatable({label = label}, mt) end
        mt.__concat = function(a, b) return wrap("(" .. a.label .. b.label .. ")") end
        local a, b, c = wrap("a"), wrap("b"), wrap("c")
        return (a .. b).label, (a .. b .. c).label, ((a .. b) .. c).label
    "#,
    );
}

#[test]
fn for_control_variables_and_iterator_aliases() {
    agrees(
        r#"
        local count, total, words = 0, 0, ""
        for i = 1, 5 do count += i; i = 7; total += i end
        for word in string.gmatch("Hello Lua user", "%a+") do words ..= word .. ":" end
        local ip = ipairs
        local values = {2, 4}
        local function sum()
            local n = 0
            for k, v in ip(values) do n += k * v end
            return n
        end
        local before = sum()
        values.extra = 3
        ip = pairs
        local after = 0
        for _, v in ip(values) do after += v end
        return count, total, words, before, after
    "#,
    );
    agrees(
        r#"
        ipairs = pairs
        unknown = pairs
        local n = 0
        for _, v in ipairs({field = 5}) do n += v end
        for _, v in unknown({field = 4}) do n += v end
        for _, v in next, {field = 2} do n += v end
        return n
    "#,
    );
}

#[test]
fn gmatch_distinguishes_letters_from_alphanumeric_runs() {
    agrees(
        r#"
        local function collect(pattern)
            local words = {}
            for word in string.gmatch("a1B_cc42!éZ", pattern) do table.insert(words, word) end
            return table.concat(words, ",")
        end
        local letters, words = collect("%a+"), collect("%w+")
        assert(letters == "a,B,cc,Z" and words == "a1B,cc42,Z")
        return letters, words
    "#,
    );
}

#[test]
fn table_constructors_and_index_constants() {
    agrees(
        r#"
        local function make(...) return {1, 2, 3, ...} end
        local a = 9
        a = {a}
        local long = {1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17}
        local tail = make(4, 5)
        local one, two, three = {a = 1, b = 2}, {b = 3, a = 4}, {a = 5, b = 6}
        one.a = 7
        return a[1], #long, long[17], #tail, tail[4], tail[5], one.a, two.a, three.a
    "#,
    );
    agrees(
        r#"
        local a, b = "key", "value"
        local strings = {[a] = 42, [b] = 0}
        local c, d = 1, 2
        local nums = {[c] = 42, [d] = 0}
        local mixed = {key = 1, value = 2, [1] = 42}
        local sparse = {[2] = 2, [256] = 256, [0] = 0, [257] = 257}
        local overlap = {[1] = 1, [2] = 2, 3}
        return strings.key, strings.value, nums[1], nums[2], mixed.key, mixed[1],
            sparse[2], sparse[256], sparse[0], sparse[257], overlap[1], overlap[2]
    "#,
    );
}

#[test]
fn writes_after_table_allocation() {
    agrees(
        r#"
        local t = setmetatable({}, nil)
        t.a, t.b, t.c, t.d, t.e, t.f, t.g, t.h, t.i = 1,1,1,1,1,1,1,1,1
        for i = 1, 10 do t[i] = 0; t.x = i end
        t.field1, t.field2 = 1, 2
        local keys = 0
        for _ in pairs(t) do keys += 1 end
        return keys, t.a, t.i, t.x, t.field1, t.field2, #t, t[4]
    "#,
    );
}

#[test]
fn conditional_comparisons_and_logical_chains() {
    agrees(
        r#"
        local function compare(a, b)
            local s = ""
            if a < b then s ..= "<" end
            if a <= b then s ..= "l" end
            if a > b then s ..= ">" end
            if a >= b then s ..= "g" end
            if a == b then s ..= "=" end
            if a ~= b then s ..= "!" end
            if not (not (a < b)) then s ..= "+" end
            if not (not (not (a < b))) then s ..= "-" end
            return s
        end
        local function chain(a, b, c, d)
            local s = ""
            if (a or b) and not (c and d) then s ..= "a" end
            if a or not b or c then s ..= "b" end
            if a and not b and c then s ..= "c" end
            return s
        end
        return compare(1,2), compare(2,2), compare(3,2),
            chain(true,false,true,false), chain(false,true,false,true), chain(false,true,true,true)
    "#,
    );
    agrees(
        r#"
        local function material(turbulence, water, bank)
            return (1 - turbulence < water + .015 and "water")
                or (bank > 0 and bank < 1 and "bank") or "stone"
        end
        return material(.9, .5, .5), material(0, .5, .5), material(0, .5, 1)
    "#,
    );
}

#[test]
fn branch_expressions_and_unary_operators() {
    agrees(
        r#"
        local function choose(a, b, c)
            result = if a then 10 elseif b then 20 elseif c then 30 else 40
            return result
        end
        local t = if true then {} else error()
        local u = if false then error() else {}
        return choose(true,false,false), choose(false,true,false), choose(false,false,true),
            choose(false,false,false), t == u, 7 + if true then 10 else 20, if nil then 10 else 20
    "#,
    );
    agrees("local function test(a,b,c) return not a,-b,#c end return test(false,2,'abc')");
}

#[test]
fn interpolated_constants_and_live_locals() {
    agrees(
        r#"
        local a, b, c = nil, "um", "uh oh"
        a = `foo{42}`
        local empty, world, n, str = "", "world", 42, "%s%s%s"
        return `hello` == "hello", `hello, {42}!`, a, b, c, `{empty}`,
            `hello, {world}!`, `hello, {world} {n}!`, `hello, {str} {n}!`
    "#,
    );
}

#[test]
fn folded_scalar_arithmetic_lengths_and_comparisons() {
    agrees("return 10+2,10-2,10*2,10/2,10%2,10^2,-(2-5),(2+2)*2");
    agrees("return #'string', #'', #'a', #('b')");
    agrees("return 1<1,1<2,1<=1,1<=2,1>1,1>2,1>=1,1>=2");
    agrees("return nil==1,nil~=1,nil==nil,nil~=nil,2==1,2~=1,1==1,1~=1");
    agrees(
        "return true==false,true~=false,true==true,true~=true,'a'=='b','a'~='b','a'=='a','a'~='a'",
    );
}

#[test]
fn folded_vector_arithmetic_and_components() {
    agrees(
        r#"
        local a, b = vector.create(1, 2, 3), vector.create(2, 4, 8)
        local function xyz(v) return v.x == v.X and v.y == v.Y and v.z == v.Z end
        local results = {a+b,a-b,a*2,a*b,2*b,a*math.huge,a/2,a/b,2/b,a/math.huge,a//2,a//b,2//b,-a}
        local parts = {}
        for _, v in ipairs(results) do
            assert(xyz(v))
            table.insert(parts, v.X)
            table.insert(parts, v.Y)
            table.insert(parts, v.Z)
        end
        return table.unpack(parts)
    "#,
    );
}

#[test]
fn folded_logical_values_and_conditions() {
    agrees(
        r#"
        global = 3
        local function choose(a, b)
            a = a and b
            b = b or global
            return a, b
        end
        local first, second = choose(false, nil)
        local third, fourth = choose(1, 2)
        return first, second, third, fourth
    "#,
    );
    agrees(
        r#"
        local a, b = nil, "right"
        return true and 2,false and 2,nil and 2,true or 2,false or 2,nil or 2,
            true and a,false and a,true or a,false or a,a and true and b,a or false or b,
            a and false,b or true
    "#,
    );
    agrees(
        r#"
        local function branch(a)
            local s = ""
            if false or a then s ..= "a" end
            if not (false or a) then s ..= "b" end
            if true and a then s ..= "c" end
            if not (true and a) then s ..= "d" end
            return s
        end
        return branch(false), branch(true), branch(0)
    "#,
    );
}

#[test]
fn constant_flow_preserves_effects_and_terminating_branches() {
    agrees(
        r#"
        local reads, visits = 0, 0
        local function probe() reads += 1 return true end
        if false then error() else visits += 1 end
        while false do error() end
        repeat visits += 1 until true
        repeat
            visits += 1
            if visits == 5 then break end
        until probe() and false
        local function finish() do if true then return 42 end end error() end
        local function stop() while true do if true then break end error() end return 43 end
        local function leave() while true do if true then return 44 end error() end end
        return reads, visits, finish(), stop(), leave()
    "#,
    );
}

#[test]
fn repeat_conditions_and_continues() {
    agrees(
        r#"
        local n, trace = 0, ""
        repeat
            n += 1
            local r = n
            repeat if r == 1 then continue end trace ..= "i" until true
            if r < 3 then continue end
            trace ..= "o"
        until (function() local a = r return a >= 3 end)()
        local outer = true
        local function test(f)
            repeat do continue end until f and outer
            return "done"
        end
        return n, trace, test(true)
    "#,
    );
    agrees(
        r#"
        local turns, hits = 0, 0
        repeat
            turns += 1
            if turns < 2 then continue else end
            hits += 1
            break
        until false
        return turns, hits
    "#,
    );
}

#[test]
fn repeat_condition_rejects_skipped_locals() {
    rejects("repeat if take then continue end local rr = 1 until rr < 2");
    rejects(
        "repeat if take then continue end local rr = take until (function() return rr end)() < 2",
    );
    rejects("repeat do continue end local c = true until c");
    rejects(
        "for i = 1, 2 do repeat if i == 2 then continue end local x = i == 1 or a until f(x) end",
    );
}

#[test]
fn repeat_continues_close_the_right_captures() {
    agrees(
        r#"
        local n, inner, body = 0, {}, {}
        repeat
            n += 1
            local b = n
            do
                local c = n * 10
                inner[n] = function() return c end
                body[n] = function() return b end
                if n < 3 then continue end
            end
        until (function() b += 1 return n == 3 end)()
        return inner[1](), inner[2](), inner[3](), body[1](), body[2](), body[3]()
    "#,
    );
    agrees(
        r#"
        local n, y = 0, nil
        repeat
            n += 1
            local a, b = n == 2, 1
            if a then continue end
            local c, d = 2, 3
            y = function() return a, b + c + d end
            c = 4
        until a
        return y()
    "#,
    );
}

#[test]
fn short_circuit_filters_and_nested_jumps() {
    agrees(
        r#"
        local function ridge(value, cutoff)
            local cutoff = cutoff or .5
            value -= cutoff
            return 1 - (value < 0 and -value or value) / (1 - cutoff)
        end
        local function thin(g, t) return g > .5 - t*.4 and g < .5 + t*.4 end
        local function thick(g, t) return g < .5 - t*.4 or g > .5 + t*.4 end
        local function saturate(x) return x < 0 and 0 or x > 1 and 1 or x end
        local function surface(x) return x and 1 or 0 end
        return ridge(.25), ridge(.75,.25), thin(.5,1), thin(1,1), thick(.5,1),
            thick(1,1), saturate(-2), saturate(.5), saturate(2), surface(false), surface(0)
    "#,
    );
    agrees(
        r#"
        local trace = ""
        local function nested(a,b)
            if a then if b then b() else end else end
            trace ..= "d"
        end
        local function b() trace ..= "b" end
        nested(true,b) nested(false,b) nested(true,nil)
        local function sample(raw,seed,scale)
            local seed, scale = seed or 0, scale or 1
            if not raw then return (2 / scale + seed * 17) * .5 + .5
            else return 2 / scale + seed * 17 end
        end
        return trace, sample(false), sample(true,1,2)
    "#,
    );
}
