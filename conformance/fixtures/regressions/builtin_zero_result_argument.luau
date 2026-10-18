-- Zero returned values supply no argument; parentheses would instead supply nil.
local function empty()
end
return 'wrong:' .. type(empty())
