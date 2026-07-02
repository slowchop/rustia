-- Derived from Luau 0.709 tests/conformance/literals.luau coroutine.wrap/yield generator.
local alphabet = {"=", "["}
local function gen(prefix, remaining)
    if remaining == 0 then
        coroutine.yield(prefix)
    else
        for _, value in pairs(alphabet) do
            gen(prefix .. value, remaining - 1)
        end
    end
end

local count = 0
local last = nil
for value in coroutine.wrap(function()
    gen("", 2)
end) do
    count += 1
    last = value
end

return count .. ":" .. last
