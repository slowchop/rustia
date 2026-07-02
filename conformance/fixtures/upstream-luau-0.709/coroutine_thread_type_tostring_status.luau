local co = coroutine.create(function()
	return 1
end)

assert(type(co) == "thread")
assert(coroutine.status(co) == "suspended")
assert(string.find(tostring(co), "thread"))

return "OK"
