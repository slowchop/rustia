-- Derived from Luau 0.709 tests/conformance/errors.luau function-target lineerror case.
a = {}

local function lineerror(s)
    local ok, msg = pcall(loadstring(s))
    local line = string.match(msg, ":(%d+):")
    return line and line + 0
end

return lineerror("function a.x.y ()\na=a+1\nend")
