-- Original regression; exact results captured with pinned Luau 0.709.
do
    local function run()
        local t={m=function(self,...) return 'old' end}; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 1,2 end; local function pass(...) return ... end; return t:m(arg())
    end
    assert(run() == "new2", "method order case 0")
end
do
    local function run()
        local t={m=function(self,...) return 'old' end}; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 1,2 end; local function pass(...) return ... end; local x=t:m(arg()); return x
    end
    assert(run() == "new2", "method order case 1")
end
do
    local function run()
        local t={m=function(self,...) return 'old' end}; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 1,2 end; local function pass(...) return ... end; local x,y=t:m(arg()); return x
    end
    assert(run() == "new2", "method order case 2")
end
do
    local function run()
        local t={m=function(self,...) return 'old' end}; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 1,2 end; local function pass(...) return ... end; return ({t:m(arg())})[1]
    end
    assert(run() == "new2", "method order case 3")
end
do
    local function run()
        local t={m=function(self,...) return 'old' end}; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 1,2 end; local function pass(...) return ... end; return pass(t:m(arg()))
    end
    assert(run() == "new2", "method order case 4")
end
do
    local function run()
        local t={m=function(self,...) return 'old' end}; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 1,2 end; local function pass(...) return ... end; return pass(pass(pass(pass(t:m(arg())))))
    end
    assert(run() == "new2", "method order case 5")
end
do
    local function run()
        local t={m=function(self,...) return 'old' end}; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 1,2 end; local function pass(...) return ... end; return t:m(pass(pass(pass(arg()))))
    end
    assert(run() == "new2", "method order case 6")
end
do
    local function run()
        local t={m=function(self,...) return 'old' end}; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 1,2 end; local function pass(...) return ... end; return t:m((arg()))
    end
    assert(run() == "new1", "method order case 7")
end
do
    local function run()
        local t={m=function(self,...) return 'old' end}; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 1,2 end; local function pass(...) return ... end; return t:m(arg(),3)
    end
    assert(run() == "new2", "method order case 8")
end
do
    local function run()
        local t={m=function(self,...) return 'old' end}; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 1,2 end; local function pass(...) return ... end; local function run(...) return t:m(arg(),...) end; return run(4,5)
    end
    assert(run() == "new3", "method order case 9")
end
do
    local function run()
        local t={m=function(self,...) return 'old' end}; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 1,2 end; local function pass(...) return ... end; local function run(...) local x=t:m(arg(),...); return x end; return run(4,5)
    end
    assert(run() == "new3", "method order case 10")
end
do
    local function run()
        local t={m=function(self,...) return 'old' end}; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 1,2 end; local function pass(...) return ... end; return t.m(t,arg())
    end
    assert(run() == "old", "method order case 11")
end
do
    local function run()
        local t={m=function(self,...) return 'old' end}; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 1,2 end; local function pass(...) return ... end; return (t.m)(t,arg())
    end
    assert(run() == "old", "method order case 12")
end
do
    local function run()
        local t={m=function() return 'old' end}; local function arg() coroutine.yield('argument'); t.m=function() coroutine.yield('method'); return 'new' end; return 1,2 end; local c=coroutine.create(function() return t:m(arg()) end); local a,b=coroutine.resume(c); local d,e=coroutine.resume(c); local f,g=coroutine.resume(c); return tostring(a)..':'..b..':'..tostring(d)..':'..e..':'..tostring(f)..':'..g
    end
    assert(run() == "true:argument:true:method:true:new", "method order case 13")
end
do
    local t = {}
    local observed = 0
    local function arg() observed = observed + 1; return 1, 2 end
    local function pass(...) return ... end
    local ok, message = pcall(function() return t:missing() end)
    local expected = "attempt to call missing method 'missing' of table"
    assert(ok == false and message:sub(-#expected) == expected)
    assert(observed == 0)
end
do
    local t = {}
    local observed = 0
    local function arg() observed = observed + 1; return 1, 2 end
    local function pass(...) return ... end
    local ok, message = pcall(function() return t:missing(arg()) end)
    local expected = "attempt to call missing method 'missing' of table"
    assert(ok == false and message:sub(-#expected) == expected)
    assert(observed == 1)
end
do
    local t = {}
    local observed = 0
    local function arg() observed = observed + 1; return 1, 2 end
    local function pass(...) return ... end
    local ok, message = pcall(function() local x=t:missing(arg()); return x end)
    local expected = "attempt to call missing method 'missing' of table"
    assert(ok == false and message:sub(-#expected) == expected)
    assert(observed == 1)
end
do
    local t = {}
    local observed = 0
    local function arg() observed = observed + 1; return 1, 2 end
    local function pass(...) return ... end
    local ok, message = pcall(function() local x,y=t:missing(arg()); return x end)
    local expected = "attempt to call missing method 'missing' of table"
    assert(ok == false and message:sub(-#expected) == expected)
    assert(observed == 1)
end
do
    local t = {}
    local observed = 0
    local function arg() observed = observed + 1; return 1, 2 end
    local function pass(...) return ... end
    local ok, message = pcall(function() return ({t:missing(arg())})[1] end)
    local expected = "attempt to call missing method 'missing' of table"
    assert(ok == false and message:sub(-#expected) == expected)
    assert(observed == 1)
end
do
    local t = {}
    local observed = 0
    local function arg() observed = observed + 1; return 1, 2 end
    local function pass(...) return ... end
    local ok, message = pcall(function() return pass(pass(pass(t:missing(arg())))) end)
    local expected = "attempt to call missing method 'missing' of table"
    assert(ok == false and message:sub(-#expected) == expected)
    assert(observed == 1)
end
do
    local t = {}
    local observed = 0
    local function arg() observed = observed + 1; return 1, 2 end
    local function pass(...) return ... end
    local ok, message = pcall(function() return t:missing((arg())) end)
    local expected = "attempt to call missing method 'missing' of table"
    assert(ok == false and message:sub(-#expected) == expected)
    assert(observed == 1)
end
do
    local t = {}
    local observed = 0
    local function arg() observed = observed + 1; return 1, 2 end
    local function pass(...) return ... end
    local ok, message = pcall(function() local function run(...) return t:missing(arg(),...) end; return run(3,4) end)
    local expected = "attempt to call missing method 'missing' of table"
    assert(ok == false and message:sub(-#expected) == expected)
    assert(observed == 1)
end
do
    local function run()
        local t={m=function(self,...) return select('#',...) end}; local function run(...) return ({t:m(...)})[1] end; return tostring(run(1,2))
    end
    assert(run() == "2", "append_varargs")
end
do
    local function run()
        local log=''; local t=setmetatable({}, {__index=function() log=log..'lookup'; return function() end end}); local function arg() log=log..'argument'; error('stop',0) end; local ok,e=pcall(function() return t:m(arg()) end); return log..':'..tostring(ok)..':'..e
    end
    assert(run() == "argument:false:stop", "argument_error")
end
do
    local function run()
        local original={m=function(self) return 'original' end}; local t=original; local function arg() t={m=function() return 'replacement' end}; original.m=function(self) return self==original and 'captured' or 'wrong' end; return 1,2 end; return t:m(arg())
    end
    assert(run() == "replacement", "receiver_capture")
end
do
    local function run()
        local function receiver() return setmetatable({}, {__index=function(self,key) if collectgarbage then collectgarbage('collect') end; return function(self,...) return key..':'..select('#',...) end end}) end; local function arg() if collectgarbage then collectgarbage('collect') end; return 1,2 end; return receiver():a_long_method_name_for_gc(arg())
    end
    assert(run() == "a_long_method_name_for_gc:2", "gc_lookup")
end
do
    local function run()
        local t={}; local function arg() coroutine.yield('arg'); return 1,2 end; local c=coroutine.create(function() local ok,e=pcall(function() return t:missing(arg()) end); local expected="attempt to call missing method 'missing' of table"; return tostring(ok)..':'..tostring(e:sub(-#expected)==expected) end); local a,b=coroutine.resume(c); if collectgarbage then collectgarbage('collect') end; local d,e=coroutine.resume(c); return tostring(a)..':'..b..':'..tostring(d)..':'..e
    end
    assert(run() == "true:arg:true:false:true", "protected_resume")
end
do
    local function run()
        local t={}; t.m=function(self,...) return 'm'..select('#',...) end; t.n=function(self,...) return 1,2 end; local function arg() t.m=function(self,...) return 'new'..select('#',...) end; return 3,4 end; return t:m(t:n(arg()))
    end
    assert(run() == "new2", "nested_method")
end
do
    local function run()
        local c=coroutine.create(function() local t={m=function() return 'old' end}; local function arg() coroutine.yield('argument'); t={m=function() return 'new' end}; return 1,2 end; return t:m(arg()) end); local a,b=coroutine.resume(c); if collectgarbage then collectgarbage('collect') end; local d,e=coroutine.resume(c); return tostring(a)..':'..b..':'..tostring(d)..':'..e
    end
    assert(run() == "true:argument:true:new", "local_after_suspend")
end
do
    local function run()
        local t={m=function() return 'old' end}; local function arg() t={m=function() return 'new' end}; return 1,2 end; local function run() return t:m(arg()) end; return run()
    end
    assert(run() == "old", "upvalue_capture")
end
do
    local function run()
        local t={m=function() return 'old' end}; local function arg() t={m=function() return 'new' end}; return 1,2 end; return (t):m(arg())
    end
    assert(run() == "new", "parenthesized_local")
end
return "pass"
