-- Original regression; expectations captured with the pinned Luau 0.709 oracle.
local function yielding() return coroutine.yield("pause") end
local cases = {
    function() local t=setmetatable({}, {__index=yielding}); return t.x end,
    function() local t=setmetatable({}, {__index=yielding}); return t:x() end,
    function() local t=setmetatable({}, {__newindex=yielding}); t.x=1 end,
    function() local t=setmetatable({}, {__add=yielding}); return t+1 end,
    function() local t=setmetatable({}, {__len=yielding}); return #t end,
    function() local t=setmetatable({}, {__tostring=yielding}); return tostring(t) end,
    function() table.sort({2,1}, yielding) end,
    function() local t=setmetatable({}, {__iter=yielding}); for k in t do end end,
    function() local t=setmetatable({}, {__concat=yielding}); return t..t end,
    function() local mt={__eq=yielding}; return setmetatable({},mt)==setmetatable({},mt) end,
}
for _, run in ipairs(cases) do
    local c = coroutine.create(run)
    local ok, message = coroutine.resume(c)
    assert(ok == false)
    assert(message == "attempt to yield across metamethod/C-call boundary")
    local again, dead = coroutine.resume(c)
    assert(again == false and dead == "cannot resume dead coroutine")
end
return "pass"
