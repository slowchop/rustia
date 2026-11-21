-- Luau 0.709 tests/Parser.test.cpp function attributes, adapted for execution.
-- MIT; see conformance/upstream/luau-0.709/LICENSE.txt.
@[deprecated{use = "plus", reason = "renamed"}]
local function oldPlus(x, y)
    return x + y
end

@native @checked
function plus(x, y)
    return oldPlus(x, y)
end

local twice = @native function(x) return plus(x, x) end
local record = {type = 'score', value = twice(6)}
assert(record.type == 'score' and record.value == 12)
return "OK"
