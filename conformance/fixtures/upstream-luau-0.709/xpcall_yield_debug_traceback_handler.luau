-- Derived from Luau 0.709 tests/conformance/pcall.luau deferred xpcall debug.traceback handler case.
local co = coroutine.create(function()
    return xpcall(function() coroutine.yield() error("foo") end, debug.traceback)
end)

assert(coroutine.resume(co))

local done, ok, trace = coroutine.resume(co)
local source = "xpcall_yield_debug_traceback_handler.luau:3"
local message = source .. ": foo"
local frame = source .. "\n"
local _, message_end = string.find(trace, message, 1, true)
local first_frame = message_end and string.find(trace, frame, message_end + 1, true)
local second_frame = first_frame and string.find(trace, frame, first_frame + 1, true)

return done and not ok and second_frame ~= nil
