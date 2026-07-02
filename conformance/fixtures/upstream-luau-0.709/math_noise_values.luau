local function noinline(x, ...)
    local _, result = pcall(function(y)
        return y
    end, x)
    return result
end

assert(math.noise(0.5) == 0)
assert(math.noise(0.5, 0.5) == -0.25)
assert(math.noise(0.5, 0.5, -0.5) == 0.125)
assert(math.noise(455.7204209769105, 340.80410508750134, 121.80087666537628) == 0.5010709762573242)

assert(math.noise(noinline(0.5)) == 0)
assert(math.noise(noinline(0.5), 0.5) == -0.25)
assert(math.noise(noinline(0.5), 0.5, -0.5) == 0.125)
assert(math.noise(noinline(455.7204209769105), 340.80410508750134, 121.80087666537628) == 0.5010709762573242)

return "OK"
