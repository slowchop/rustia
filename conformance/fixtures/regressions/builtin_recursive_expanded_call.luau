local function run(n)
    if n == 0 then return 4, 5 end
    return string.len('abc', run(n - 1))
end
string.len = function(...) return select('#', ...), 7 end
return tostring(run(20))
