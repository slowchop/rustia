-- Derived from Luau 0.709 tests/conformance/literals.luau control-character scanner assertion.
if string.find("\a\b\f\n\r\t\v", "^%c%c%c%c%c%c%c$") == 1 then
    return "OK"
end
return "BAD"
