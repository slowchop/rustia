math.randomseed(0)
assert(math.random(1000000) == 782827)
assert(math.random(1000000) == 551456)
assert(math.random(1000000) == 107634)

math.randomseed(12345)
assert(math.random(-10, 10) == 8)
assert(math.random(-10, 10) == -2)
assert(math.random(5) == 1)

math.randomseed(-1)
assert(math.random(1000000) == 586250)
assert(math.random(-10, 10) == -10)

return "OK"
