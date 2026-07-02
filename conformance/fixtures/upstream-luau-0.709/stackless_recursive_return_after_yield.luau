local count = 0

local function f(n)
    if n == 0 then
        return coroutine.yield("pause")
    end

    local value = f(n - 1)
    count += 1
    return value
end

local co = coroutine.create(function()
    return f(3)
end)

local ok, yielded = coroutine.resume(co)
if ok ~= true or yielded ~= "pause" then
    return false
end

local ok2, result = coroutine.resume(co, "done")
return ok2 == true and result == "done" and count == 3
