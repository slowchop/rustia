local v, e = math.frexp(1.5)
assert(v == 0.75 and e == 1)

local sv, se = math.frexp("1.5")
assert(sv == 0.75 and se == 1)

assert(select("#", math.frexp(1.5)) == 2)
assert(select(1, math.frexp(1.5)) == 0.75)
assert(select(2, math.frexp(1.5)) == 1)
local pv, pe = math.frexp(math.pi)
assert(math.ldexp(pv, pe) == math.pi)

return "OK"
