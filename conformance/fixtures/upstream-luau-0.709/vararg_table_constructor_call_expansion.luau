local function f()
    return 1, 2, 3
end

local t = { f() }
return table.getn(t) * 100 + t[3]
