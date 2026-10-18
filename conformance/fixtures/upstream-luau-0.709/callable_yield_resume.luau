-- Luau 0.709 keeps callable self and ordinary callers across suspension.
local receiver = {}
setmetatable(receiver, {__call = function(self, n)
    assert(self == receiver)
    local answer = coroutine.yield('waiting')
    return n + answer
end})
local owner = coroutine.create(function() return receiver(20) end)
local ok, value = coroutine.resume(owner)
assert(ok and value == 'waiting')
ok, value = coroutine.resume(owner, 22)
return ok and value == 42 and coroutine.status(owner) == 'dead'
