-- Derived from Luau 0.709 tests/conformance/utf8.luau position-capture charpattern loop.
local x = "日本語a-4\0éó"
local seen = {}

for p, c in string.gmatch(x, "()(" .. utf8.charpattern .. ")") do
    table.insert(seen, tostring(p) .. ":" .. tostring(#c))
end

return table.concat(seen, ",")
