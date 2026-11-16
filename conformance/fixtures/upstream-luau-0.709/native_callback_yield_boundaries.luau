-- Derived from the pinned Luau 0.709 coroutine boundary checks.
local boundary = "attempt to yield across metamethod/C-call boundary"

local function nestedCoroutineCanYield()
    local seen
    local child = coroutine.create(function()
        seen = coroutine.isyieldable()
        coroutine.yield("ok")
        return "done"
    end)
    local firstOk, firstValue = coroutine.resume(child)
    local secondOk, secondValue = coroutine.resume(child)
    local failing = coroutine.create(function()
        error("child failure", 0)
    end)
    local failureOk, failure = coroutine.resume(failing)
    return seen == true and firstOk == true and firstValue == "ok"
        and secondOk == true and secondValue == "done"
        and failureOk == false and failure == "child failure"
end

local tostringSeen, tostringOk, tostringError
local value = setmetatable({}, {
    __tostring = function()
        assert(nestedCoroutineCanYield())
        tostringSeen = coroutine.isyieldable()
        tostringOk, tostringError = pcall(coroutine.yield, "pause")
        return "value"
    end,
})
assert(tostring(value) == "value")
assert(tostringSeen == false)
assert(tostringOk == false and tostringError == boundary)

local gsubSeen, gsubOk, gsubError
local replaced = string.gsub("a", ".", function()
    assert(nestedCoroutineCanYield())
    gsubSeen = coroutine.isyieldable()
    gsubOk, gsubError = pcall(coroutine.yield, "pause")
    return "b"
end)
assert(replaced == "b")
assert(gsubSeen == false)
assert(gsubOk == false and gsubError == boundary)

local foreachSeen, foreachOk, foreachError
table.foreach({ key = "value" }, function()
    assert(nestedCoroutineCanYield())
    foreachSeen = coroutine.isyieldable()
    foreachOk, foreachError = xpcall(coroutine.yield, function(message)
        return message
    end, "pause")
end)
assert(foreachSeen == false)
assert(foreachOk == false and foreachError == boundary)

local foreachiSeen, foreachiOk, foreachiError
table.foreachi({ "value" }, function()
    assert(nestedCoroutineCanYield())
    foreachiSeen = coroutine.isyieldable()
    foreachiOk, foreachiError = pcall(coroutine.yield, "pause")
end)
assert(foreachiSeen == false)
assert(foreachiOk == false and foreachiError == boundary)

local directCases = {
    function()
        return tostring(setmetatable({}, { __tostring = coroutine.yield }))
    end,
    function()
        return string.gsub("a", ".", coroutine.yield)
    end,
    function()
        return table.foreach({ key = "value" }, coroutine.yield)
    end,
    function()
        return table.foreachi({ "value" }, coroutine.yield)
    end,
}
for _, run in ipairs(directCases) do
    local thread = coroutine.create(run)
    local ok, message = coroutine.resume(thread)
    assert(ok == false and message == boundary)
    local again, dead = coroutine.resume(thread)
    assert(again == false and dead == "cannot resume dead coroutine")
end

return "pass"
