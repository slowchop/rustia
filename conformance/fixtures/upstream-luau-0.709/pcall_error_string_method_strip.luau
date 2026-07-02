-- Derived from Luau 0.709 tests/conformance/assert.luau ecall helper.
local ok, err = pcall(function()
    error("oops")
end)

if ok then
    return "unexpected success"
end
return err:sub(err:find(": ") + 2, #err)
