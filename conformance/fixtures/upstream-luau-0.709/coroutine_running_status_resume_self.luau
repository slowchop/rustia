local co

co = coroutine.create(function()
	assert(coroutine.running() == co)
	assert(coroutine.status(co) == "running")
	local ok, message = coroutine.resume(co)
	assert(ok == false)
	assert(string.find(message, "running"))
	assert(coroutine.isyieldable())
	return "done"
end)

local ok, result = coroutine.resume(co)
assert(ok and result == "done")

return coroutine.status(co)
