-- Derived from Luau 0.709 tests/conformance/pcall.luau xpcall debug.traceback handler case.
local ok, trace = xpcall(function() error("foo") end, debug.traceback)

local source = "xpcall_debug_traceback_handler.luau:2"
local message = source .. ": foo"
local frame = source .. "\n"
local _, message_end = string.find(trace, message, 1, true)
local first_frame = message_end and string.find(trace, frame, message_end + 1, true)
local second_frame = first_frame and string.find(trace, frame, first_frame + 1, true)

return not ok and second_frame ~= nil
