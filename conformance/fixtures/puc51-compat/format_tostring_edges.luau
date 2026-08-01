-- PUC Lua 5.1.5 differential fixture: %.14g number formatting edge cases
-- through four in-guest channels (tostring, .., table.concat, string.format
-- "%s"): exponent boundaries X in {-5,-4,13,14}, the 1e21 fixed/exponent
-- boundary, round-carry flip, decimal tie, -0, subnormal, DBL_MIN/MAX.
-- Expectation captured offline from the pinned 5.1.5 oracle.
local parts = {}
local function emit(s)
    parts[#parts + 1] = s
end
local vals = {
    -0.0,
    0.1,
    1e-5,
    0.0001,
    1e13,
    1e14,
    1e15,
    1e20,
    1e21,
    99999999999999.9,
    100000000000000.5,
    1000000000000000.5,
    123456789012345678,
    5e-324,
    2.2250738585072014e-308,
    1.7976931348623157e308,
    0.5,
    1.5,
    2 ^ 53,
    2 ^ 53 + 2,
    3.141592653589793,
}
for i = 1, #vals do
    local v = vals[i]
    emit(
        i
            .. "|"
            .. tostring(v)
            .. "|"
            .. ("x" .. v)
            .. "|"
            .. table.concat({ v, v }, "~")
            .. "|"
            .. string.format("%s", v)
    )
end
return table.concat(parts, ";")
