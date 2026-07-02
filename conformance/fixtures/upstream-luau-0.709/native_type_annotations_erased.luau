-- Derived from Luau 0.709 tests/conformance/native_types.luau annotation coverage.
-- Native JIT guard behavior is intentionally not asserted here; annotations are runtime-erased.
local function optstring(s: string?)
    return if s then s .. "2" else "3"
end

local function checktable(a: {x:number})
    return a.x
end

local function checkfunction(a: () -> ())
    return a()
end

local function checkthread(a: thread)
    return type(a)
end

local function checkuserdata(a: userdata)
    return type(a)
end

local function checkvector(a: vector)
    return typeof(a)
end

local function checkbuffer(a: buffer?)
    return if a then type(a) else "nil"
end

local thread = coroutine.create(function()
end)
local userdata = newproxy()
local buffer_value = buffer.create(2)

assert(optstring(nil) == "3")
assert(optstring("two: ") == "two: 2")
assert(checktable({x = 7}) == 7)
assert(checkfunction(function()
    return 9
end) == 9)
assert(checkthread(thread) == "thread")
assert(checkuserdata(userdata) == "userdata")
assert(checkvector(vector.create(1, 2, 3)) == "vector")
assert(checkbuffer(buffer_value) == "buffer")
assert(checkbuffer(nil) == "nil")

return "OK"
