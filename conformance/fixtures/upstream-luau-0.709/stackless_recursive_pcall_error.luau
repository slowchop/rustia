local caught = false

local function f(n)
    if n == 0 then
        error("boom")
    end

    local ok, err = pcall(f, n - 1)
    if not ok and string.match(tostring(err), "boom") ~= nil then
        caught = true
    end
    return "ok"
end

local outer_ok, result = pcall(f, 3)
return outer_ok == true and result == "ok" and caught == true
