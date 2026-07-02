local function pack(...)
	return table.pack(...)
end

local function f()
	return 1, 2, 30, 4
end

local function ret2(a, b)
	return a, b
end

local function unlpack(t, i)
	i = i or 1
	if i <= table.getn(t) then
		return t[i], unlpack(t, i + 1)
	end
end

local a, b, c, d = unlpack(pack(ret2(f()), ret2(f())))

assert(a == 1, "a")
assert(b == 1, "b")
assert(c == 2, "c")
assert(d == nil, "d")

return "OK"
