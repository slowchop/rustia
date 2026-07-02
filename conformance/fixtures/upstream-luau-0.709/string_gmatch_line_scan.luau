-- Derived from Luau 0.709 tests/conformance/errors.luau stack traceback scanning.
local stackmsg = "first:12:\n\nthird:345:\nplain"
local out = {}

for line in string.gmatch(stackmsg, "[^\n]*") do
    local curr = string.match(line, ":(%d+):")
    if curr then
        table.insert(out, curr)
    end
end

return table.concat(out, ",")
