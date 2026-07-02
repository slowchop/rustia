-- Derived from Luau 0.709 tests/conformance/basic.luau concat helper and pcall multi-return checks.
function concat(head, ...)
    if select("#", ...) == 0 then
        return tostring(head)
    else
        return tostring(head) .. "," .. concat(...)
    end
end

return concat(pcall(function()
    return 1, 2, 3
end))
