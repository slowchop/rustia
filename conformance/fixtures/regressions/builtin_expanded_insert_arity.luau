-- The trailing call supplies two values, producing four effective arguments.
local values = {}
local function many()
    return 4, 99
end
table.insert(values, 1, many())
return 'wrong: trailing results were discarded'
