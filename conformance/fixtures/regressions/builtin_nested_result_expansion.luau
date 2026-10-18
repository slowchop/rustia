local function pass(...) return ... end
local function many() return 4, 5 end
local function empty() end
local ordinary = select('#', pass(pass(pass(pass(many())))))
string.len = function(...) return select('#', ...) end
local values = {string.len('abc', pass(pass(pass(pass(many())))))}
local type_ok = pcall(function() return type(pass(pass(pass(empty())))) end)
local insert_ok = pcall(function()
    local target = {}
    local ignored = {table.insert(target, 1, pass(pass(pass(many()))))}
end)
local function varargs(...)
    return select('#', pass(pass(pass(pass(...)))))
end
return ordinary .. ':' .. values[1] .. ':' .. tostring(type_ok) .. ':' .. tostring(insert_ok) .. ':' .. varargs(4, 5)
