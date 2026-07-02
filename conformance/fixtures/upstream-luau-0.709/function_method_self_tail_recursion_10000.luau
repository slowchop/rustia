local a = {}

function a:deep(n)
	if n > 0 then
		return self:deep(n - 1)
	else
		return 101
	end
end

return a:deep(10000)
