local caught = false

local function f(n)
    if n == 0 then
        coroutine.yield("pause")
        error("boom")
    end

    local ok, err = pcall(f, n - 1)
    if not ok and string.match(tostring(err), "boom") ~= nil then
        caught = true
    end
    return "ok"
end

local co = coroutine.create(function()
    return f(3)
end)

local ok, yielded = coroutine.resume(co)
if ok ~= true or yielded ~= "pause" then
    return false
end

local ok2, result = coroutine.resume(co)
return ok2 == true and result == "ok" and caught == true
