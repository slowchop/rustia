function fat(x)
	if x <= 1 then
		return 1
	else
		return x * loadstring("return fat(" .. x - 1 .. ")")()
	end
end

local chunk = loadstring("return fat(5), 3")
local a, b = chunk()
assert(a == 120 and b == 3)

return "OK"
