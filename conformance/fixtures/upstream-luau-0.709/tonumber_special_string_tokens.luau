assert(math.isinf(tonumber("inf")))
assert(math.isinf(tonumber("+inf")))
assert(tonumber("-inf") == -math.huge)
assert(math.isinf(tonumber("infinity")))
assert(math.isinf(tonumber("  Infinity  ")))

assert(math.isnan(tonumber("nan(123)")))
assert(math.isnan(tonumber("nan(payload text)")))
assert(math.isnan(tonumber("+nan(_)")))
assert(math.isnan("nan(payload)" + 1))

assert(tonumber("nan(foo)bar") == nil)
assert(tonumber("nan(") == nil)
assert(tonumber("0b101") == nil)
assert(tonumber("1_000") == nil)
assert(tonumber("0x1_0") == nil)
assert(pcall(function()
    return "0b101" + 1
end) == false)

return "OK"
