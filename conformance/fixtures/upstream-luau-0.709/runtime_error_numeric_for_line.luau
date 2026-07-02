-- Derived from Luau 0.709 tests/conformance/errors.luau lineerror numeric-for case.
local function lineerror(s)
    local ok, msg = pcall(loadstring(s))
    local line = string.match(msg, ":(%d+):")
    return line and line + 0
end

return lineerror("local a\n for i=1,'a' do \n print(i) \n end")
