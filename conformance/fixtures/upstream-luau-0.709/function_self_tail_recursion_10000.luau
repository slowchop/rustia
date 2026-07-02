local function deep(n)
	if n > 0 then
		return deep(n - 1)
	else
		return 101
	end
end

return deep(10000)
