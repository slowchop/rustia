-- Original #18 regressions, checked against the pinned Luau 0.709 worker.
local cases = {
    {function() local x; x.y = 1 end, "attempt to index nil with 'y'"},
    {function() local x; local k = 'z'; x[k] = 1 end, "attempt to index nil with 'z'"},
    {function() local x; x[1] = 1 end, "attempt to index nil with number"},
    {function() local x; x[true] = 1 end, "attempt to index nil with boolean"},
    {function() local x; x[nil] = 1 end, "attempt to index nil with nil"},
    {function() local x; x[{}] = 1 end, "attempt to index nil with table"},
    {function() local x = vector.create(1, 2, 3); x.x = 1 end, "attempt to index vector with 'x'"},
    {function() local x = vector.create(1, 2, 3); x[1] = 1 end, "attempt to index vector with number"},
    {function() local x = buffer.create(1); x.y = 1 end, "attempt to index buffer with 'y'"},
    {function() local x = 42; x.y = 1 end, "attempt to index number with 'y'"},
    {function() local x = false; x.y = 1 end, "attempt to index boolean with 'y'"},
    {function() local x = 'text'; x.y = 1 end, "attempt to index string with 'y'"},
    {function() local x = function() end; x.y = 1 end, "attempt to index function with 'y'"},
}
for _, case in ipairs(cases) do
    local ok, message = pcall(case[1])
    assert(not ok and type(message) == 'string')
    assert(message:match('^.-:%d+: ') ~= nil)
    message = message:gsub('^.-:%d+: ', '', 1)
    assert(message == case[2], message)
end
local worker = coroutine.wrap(function()
    coroutine.yield()
    error('wrapped failure')
end)
assert(pcall(worker))
local ok, message = pcall(worker)
assert(not ok and message:match('^.-:26: wrapped failure$'), tostring(message))
return 'OK'
