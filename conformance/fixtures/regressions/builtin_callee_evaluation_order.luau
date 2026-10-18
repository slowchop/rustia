local original = string.len
local function replace()
    string.len = function(...)
        return 100 + select('#', ...)
    end
    return 99, 88
end
local expanded = '' .. string.len('abc', replace())
string.len = original
local parenthesized = '' .. (string.len)('abc', replace())
string.len = original
local alias = string.len
local aliased = '' .. alias('abc', replace())
local object = {pass = function(self, ...) return ... end}
string.len = original
local method_fixed = '' .. object:pass(string.len('abc', replace()))
string.len = original
local function method_return()
    return object:pass(string.len('abc', replace()))
end
local method_tail = '' .. method_return()
return expanded .. ':' .. parenthesized .. ':' .. aliased .. ':' .. method_fixed .. ':' .. method_tail
