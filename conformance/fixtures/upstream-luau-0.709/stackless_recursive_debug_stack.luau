-- Derived from Luau 0.709 debug.traceback/debug.info stack-frame behavior.
local function count_needles(haystack, needle)
    local count = 0
    local init = 1
    while true do
        local first, last = string.find(haystack, needle, init, true)
        if not first then
            return count
        end
        count += 1
        init = last + 1
    end
end

function recurse(n)
    if n == 0 then
        return debug.traceback("mark")
    end

    local trace = recurse(n - 1)
    return trace
end

local trace = recurse(3)
if count_needles(trace, "function recurse") < 3 then
    return false
end

function stack_info(n)
    if n == 0 then
        return debug.info(1, "n"), debug.info(2, "n"), debug.info(3, "n")
    end

    local a, b, c = stack_info(n - 1)
    return a, b, c
end

local a, b, c = stack_info(3)
return a == "stack_info" and b == "stack_info" and c == "stack_info"
