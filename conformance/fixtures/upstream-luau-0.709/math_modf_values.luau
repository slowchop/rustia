local a, b = math.modf(3.5)
assert(a == 3 and b == 0.5)

local c, d = math.modf(-3.5)
assert(c == -3 and d == -0.5)

local e, f = math.modf("1.5")
assert(e == 1 and f == 0.5)

assert(select("#", math.modf(1.5)) == 2)
assert(select(1, math.modf(math.huge)) == math.huge)
assert(select(2, math.modf(math.huge)) == 0)

local _, negzero = math.modf(-0)
assert(1 / negzero == -math.huge)

local _, negintfrac = math.modf(-2)
assert(1 / negintfrac == -math.huge)

local _, neginffrac = math.modf(-math.huge)
assert(1 / neginffrac == -math.huge)

return "OK"
