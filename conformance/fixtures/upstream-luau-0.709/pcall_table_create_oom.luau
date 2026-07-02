-- Derived from Luau 0.709 tests/conformance/pcall.luau OOM protected-call cases.
local ok, err = pcall(function()
    table.create(1e6)
end)

return ok == false and err == "not enough memory"
