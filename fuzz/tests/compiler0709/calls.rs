//! Portable cases from Luau 0.709 tests/Compiler.test.cpp:3037–5738.
//! Upstream MIT license: conformance/upstream/luau-0.709/LICENSE.txt.

use crate::{agrees, rejects};

#[test]
fn literal_table_indices() {
    agrees(
        r##"
local a = {}
local missing = select("#", a[0], a[1], a[256], a[257])
a[0], a[1], a[256], a[257] = "zero", "one", "256", "257"
a["field"] = a["one"] or 2
assert(missing == 4)
return a[0], a[1], a[256], a[257], a[255], a["field"]
"##,
    );
}

#[test]
fn nested_min_max_and_clamp_calls() {
    agrees(
        r#"
local function clamp(t,a,b) return math.min(math.max(t,a),b) end
local function fast(a,b,c) return math.min(a,b,c) + math.clamp(a,b,c) end
return clamp(-2,1,5), clamp(3,1,5), clamp(8,1,5), fast(4,1,8)
"#,
    );
}

#[test]
fn captures_escape_each_loop_and_break() {
    agrees(
        r#"
local function run(stop)
    local fs = {}
    local function keep(f) table.insert(fs, f) end
    for i=1,3 do i=i; keep(function() return i end); if i==stop then break end end
    for i in ipairs({1,2,3}) do i=i; keep(function() return i end); if i==stop then break end end
    local i=0
    while i<3 do local j=i; keep(function() return j end); i+=1; if i==stop then break end end
    i=0
    repeat local j=i; keep(function() return j end); i+=1; if i==stop then break end until i==3
    local values={}
    for n,f in ipairs(fs) do values[n]=f() end
    return table.concat(values, ",")
end
assert(run(2)=="1,2,1,2,0,1,0,1")
return run(2), run(4)
"#,
    );
}

#[test]
fn alias_on_a_local_and_constant_assertion() {
    agrees("type A = number local a: A = 1 return a, (1 + 2) :: number");
}

#[test]
fn debug_lines_in_branches_and_loops() {
    agrees(
        r#"
local entries = {Mountains=true,Canyons=true,Dunes=true,Arctic=true,Lavaflow=true,Hills=true,Plains=true,Marsh=true,Water=true}
local keys, site = {}, 0
for k in pairs(entries) do
    table.insert(keys,k)
    site = debug.info(1,"l")
end
table.sort(keys)
local called={}
local sink={record=function() table.insert(called,debug.info(2,"l")) end}
local count=0
while true do
    count+=1
    if count>1 then
        sink.record()
        break
    end
end
local f=0
repeat
    f+=1
    if f==1 then
        sink.record()
    else
        f=0
    end
until f==0
return table.concat(keys,","), site, count, f, called[1], called[2]
"#,
    );
}

#[test]
fn generic_for_iterator_source_line() {
    agrees(
        r#"
local lines={}
local function iter(state,last)
    table.insert(lines,debug.info(2,"l"))
    if last<state then return last+1 end
end
local sum=0
for
i
in
iter
,
2
,
0
do
    sum+=i
end
return sum, table.concat(lines,",")
"#,
    );
    agrees(
        r#"
local seen={}
local function nextvalue(_,last)
    seen[#seen+1]=debug.info(2,"l")
    if not last then return 1 end
end
local provider=setmetatable({}, {__iter=function()
    seen[#seen+1]=debug.info(2,"l")
    return nextvalue
end})
for
    i in provider
do assert(i==1) end
return table.concat(seen,",")
"#,
    );
    agrees(
        r#"
local function run()
    for i in
        4,
        2,
        0
    do end
end
local ok,message=pcall(run)
assert(not ok, message)
return tonumber(string.match(message,":(%d+):"))
"#,
    );
}

#[test]
fn multiline_calls_and_table_access_locations() {
    agrees(
        r#"
local sink={marker=function() return debug.info(2,"l") end}
local lines = {}
local target = setmetatable({}, {__newindex=function(t,k,v)
    lines.write=debug.info(2,"l"); rawset(t,k,v)
end})
local outer = setmetatable({}, {__index=function() lines.read=debug.info(2,"l"); return target end})
outer.SubTable["Key"] = {
    a = sink.marker(),
    b = sink.marker(),
    c = sink.marker(),
    d = true,
}
local a = {b={c=setmetatable({}, {__newindex=function() lines.deep=debug.info(2,"l") end})}}
a
    ["b"]
    ["c"]
    ["d"] = 4
local got=target.Key
return lines.read,lines.write,got.a,got.b,got.c,got.d,lines.deep
"#,
    );
    agrees(
        r#"
local calls={}
local obj={}
function obj:Bar(x,y,z) table.insert(calls,debug.info(2,"l")); return self end
function obj:Baz(x) table.insert(calls,debug.info(2,"l")); return self end
function obj.Qux(x) table.insert(calls,debug.info(2,"l")) end
obj:Bar(
    1,
    2,
    3)
obj
    :Bar(1)
    :Baz(2)
    .Qux(3)
return table.concat(calls,",")
"#,
    );
    agrees(
        r#"
local function fail()
    return
        math.max(
            "x",
            4)
end
local ok,message=pcall(fail)
assert(not ok)
local line=tonumber(string.match(message,":(%d+):"))
assert(line==4,message)
return line
"#,
    );
    agrees(
        r#"
local obj={}
function obj:method(...) return debug.info(2,"l"), ... end
local a,b =
    obj
        :method(42)
local list = {
    obj
        :method(43)
}
local wrapper = {}
function wrapper:tail(...)
    return
        obj
            :method(...)
end
local c,d=wrapper:tail(44)
return a,b,list[1],list[2],c,d
"#,
    );
}

#[test]
fn repeat_and_function_local_visibility() {
    agrees(
        r#"
local result={}
local a="outer"
local function foo(e,f)
    local a=1
    for i=1,3 do result[#result+1]=i end
    for k,v in ipairs({4,5}) do result[#result+1]=k+v end
    do local b=2; result[#result+1]=b end
    do local c=2; assert(b==nil) end
    repeat local a,b until true
    repeat local a,b; do continue end; local c,d=2 until true
    local function inner() return inner,a end
    local same,value=inner()
    return same==inner,value,e,f
end
local self,value,e,f=foo(6,7)
return table.concat(result,","),self,value,e,f,a
"#,
    );
}

#[test]
fn assignment_targets_survive_local_rebinding() {
    agrees(
        r#"
local a,b
a,b=1,2
assert(a==1 and b==2)
local old={}
a=old; a,a[1]=1,2; assert(a==1 and old[1]==2)
a=old; a[1],a=3,4; assert(old[1]==3 and a==4)
a=old; a,a.foo=5,6; assert(a==5 and old.foo==6)
local map={}
a=7; a,map[a]=8,9; assert(a==8 and map[7]==9)
a=old; a,a[a]=10,11; assert(a==10 and old[old]==11)
a,b=old,12; a,b,a[b]=13,14,15; assert(a==13 and b==14 and old[12]==15)
a=16; a,map[a+1]=18,19
return a,map[17],old[old],old[12]
"#,
    );
}

#[test]
fn builtin_bindings_select_and_reassignment() {
    agrees(
        r##"
local abs=math.abs
local function captured() return abs(-5) end
local a,b,c=math.abs(-5),abs(-5),captured()
local saved=math
math={}
local missing=pcall(function() return math.abs(-5) end)
local old=captured()
math=saved
abs=nil
local rebound=pcall(captured)
local function pack(...) return select("#",...), ... end
local function all(...) local sum=0; for i=1,select("#",...) do sum+=select(i,...) end; return sum end
local function items() return 10,nil,30 end
return a,b,c,missing,old,rebound,all(2,3,4),select("#"),select("#",items()),pack(items())
"##,
    );
}

#[test]
fn immutable_mutable_and_recursive_captures() {
    agrees(
        r#"
local function maker(a,b)
    local function observe() return a,b end
    a=1
    return observe
end
local t={val=77}
function t:unchanged() return function() return self.val end end
function t:changed() local result=function() return self end; self=42; return result end
local function nested(x)
    local y=6
    local function descend() return function() return x+y end end
    x=3
    return descend()
end
local function recursive(n)
    if n==0 then return 1 end
    local function step() return recursive(n-1) end
    return step()+1
end
local x,y=maker(4,5)()
return x,y,t:unchanged()(),t:changed()(),nested(9)(),recursive(3)
"#,
    );
}

#[test]
fn compound_lvalues_are_evaluated_once() {
    agrees(
        r#"
G=3; G+=1; local first=G; G-=G
local a=2; a*=2; local b=a; a/=a; a/=a+1
local up=5; local function mutate() up+=4 end; mutate()
local t={x=2, [1]=3}; t[t]=4
t.x+=5; t[1]+=5; t[t]+=5
local trace=""
local function object() trace..="o"; return t end
local function key() trace..="k"; return "x" end
local function rhs() trace..="r"; return 5 end
object()[key()]+=rhs()
local s=""; s..="a"; s..="b".."c"; s..="d".."e".."f"
G=""; G..="g".."h"
return first,G,b,a,up,t.x,t[1],t[t],trace,s
"#,
    );
}

#[test]
fn valid_and_invalid_compiler_input() {
    agrees("return 5");
    rejects("this is not valid lua, right?");
}

#[test]
fn chained_namecall_receivers() {
    agrees(
        r#"
local mt={}
mt.__index=mt
function mt:Method(x) return setmetatable({path=self.path*10+x},mt) end
local obj=setmetatable({path=0},mt)
return obj:Method(1):Method(2):Method(3).path,obj.path
"#,
    );
}

#[test]
fn discarded_initializers_still_run() {
    agrees(
        r#"
local calls=0
local function g() calls+=1; return 8 end
local a,b=1,2
local x=g()
local retained=1,g()
return a+b,x,retained,calls
"#,
    );
}

#[test]
fn comparisons_with_scalar_constants() {
    agrees(
        r#"
local function compare(obj)
    return (obj==1)==(1==obj), 1==obj, "Hello, Sailor!"==obj, nil==obj, true==obj, nil~=obj, obj=={}
end
for _,v in ipairs({1,"Hello, Sailor!",true,{}}) do
    local same,number,text,absent,boolean,present,fresh=compare(v)
    assert(same and number==(v==1) and text==(v=="Hello, Sailor!") and boolean==(v==true))
    assert(not absent and present and not fresh)
end
return compare(nil)
"#,
    );
}

#[test]
fn closure_captures_and_global_environment() {
    agrees(
        r#"
local function cell(value) return function(delta) value+=delta; return value end end
local left,right=cell(1),cell(1)
local saved=left
local ids={[left]="l",[right]="r"}
assert(saved==left and left~=right)
assert(ids[saved]=="l" and ids[right]=="r")
assert(left(3)==4 and right(1)==2)
app_global="old"
local function global_reader() return function() return app_global end end
local function captured_reader(x) return function() return x end end
local global=global_reader()
local captured=captured_reader(app_global)
app_global="new"
local const=11
local function transitive()
    local function read() return const end
    return function() return read() end
end
local values={}
for i=1,2 do local j=i; values[i]=function() return i+j end end
for k,v in ipairs({12,14}) do values[k+2]=function() return k+v end end
return global(),captured(),transitive()(),values[1](),values[2](),values[3](),values[4]()
"#,
    );
    agrees(
        r#"
local function factory() return function() return label end end
setfenv(factory,{label="changed"})
local created=factory()
return created()
"#,
    );
}

#[test]
fn dynamic_global_lookup() {
    agrees(
        r#"
local env=getfenv()
local log=""
local function record(x) return {print=function() log..=x end} end
for _,name in ipairs({"game","plugin","script","shared","workspace"}) do env[name]=record(name) end
game.print(); plugin.print(); script.print(); shared.print(); workspace.print()
game=record("new"); game.print()
return log
"#,
    );
}

#[test]
fn constants_and_arithmetics() {
    agrees("return nil,true,42,'hello'");
    agrees(
        r#"
local function arithmetic(a,b)
    return a+b,a-b,a/b,a*b,a%b,a^b
end
local function rhs(a)
    return a+1,a-1,a/1,a*1,a%1,a^1
end
local x,y,z,p,q,r=arithmetic(6,2)
return x,y,z,p,q,r,rhs(6)
"#,
    );
}

#[test]
fn vector_constructor_results_and_constants() {
    agrees(
        r#"
local function make(a,b,c) return vector.create(a,b,c) end
local a,b,c,d=vector.create(1,2),vector.create(1,2,3),make(4,5,6),vector.create(7,8,9,10)
local zero=vector.create(0,0,0)
local neg=vector.create(-0,0,0)
return a.x,a.y,a.z,b.x,b.y,b.z,c.x,c.y,c.z,d.x,d.y,d.z,type(b),
       1/zero.x,1/neg.x,vector.one==vector.create(1,1,1),vector.zero==zero
"#,
    );
    // Stock standalone Luau supplies vector.create. Bind the upstream fixture's
    // legacy name explicitly to exercise the same literal and dynamic calls.
    agrees(
        r#"
local Vector3={new=vector.create,one=vector.one,xAxis=vector.create(1,0,0)}
local function new(a,b,c) return Vector3.new(a,b,c) end
local fixed=Vector3.new(1,2,3)
local dynamic=new(4,5,6)
return fixed.x,fixed.y,fixed.z,dynamic.x,dynamic.y,dynamic.z,Vector3.one.z,Vector3.xAxis.x
"#,
    );
}

#[test]
fn erased_type_assertion_selects_one_result() {
    agrees(
        r##"
local calls=0
local function foo() calls+=1; return 10,20 end
local function count(...) return select("#",...) end
return count(foo() :: typeof(error("compile time"))), count(foo()), calls
"##,
    );
}

#[test]
fn annotated_arguments_and_captured_numbers() {
    agrees(
        r#"
local up: number = 2
local function foo(e: vector, f: mat3, g: sequence)
    local h = e * e
    local m = e * f
    local sum=0
    for i=1,3 do sum+=i end
    up+=g
    return h.x,m.y,sum,up
end
return foo(vector.create(2,3,4),vector.create(4,5,6),7)
"#,
    );
}

#[test]
fn value_captures_builtin_results_and_loop_bounds() {
    agrees(
        r#"
local value=true
local function get() return value end
local function invert() return not value end
local function foo(x) return (math.abs(x)) end
local function sum(limit) local s=0; for i=1,limit do s+=i end; return s end
local function test(a)
    while a<0 do a+=1 end
    for i=10,1,-1 do a+=1 end
    for i in pairs({}) do a+=1; if a%2==0 then continue end end
    repeat a+=1; if a%2==0 then break end until a>10
    return a
end
return foo(-1)+foo(assert(-3)),get(),invert(),sum(100),sum(4*25),test(-2),test(2)
"#,
    );
}
