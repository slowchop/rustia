-- VM/src/lvmutils.cpp luaV_tryfuncTM requires __call itself to be a function.
local inner = setmetatable({}, {__call = function() return true end})
local function arguments() return 1, 2 end
for _, value in ipairs({false, 42, 'call', inner}) do
    local receiver = setmetatable({}, {__call = value})
    local ok, err = pcall(function() return receiver() end)
    assert(not ok and string.find(err, 'attempt to call a table value', 1, true))
    ok, err = pcall(function() return receiver(arguments()) end)
    assert(not ok and string.find(err, 'attempt to call a table value', 1, true))
end
return true
