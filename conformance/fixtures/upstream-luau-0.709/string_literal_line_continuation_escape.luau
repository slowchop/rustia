-- Derived from Luau 0.709 tests/conformance/literals.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
local y = "\
hello\r\n\
"
if y == "\nhello\r\n\n" then
    return "OK"
end
return y
