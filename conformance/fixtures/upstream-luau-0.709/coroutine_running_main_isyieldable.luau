local main = coroutine.running()

assert(main == nil)
assert(coroutine.isyieldable())

return "OK"
