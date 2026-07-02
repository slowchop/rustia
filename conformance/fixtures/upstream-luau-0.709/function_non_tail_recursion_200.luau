local function deep(n)
	if n > 0 then
		deep(n - 1)
	end
end

deep(10)
deep(200)

return "OK"
