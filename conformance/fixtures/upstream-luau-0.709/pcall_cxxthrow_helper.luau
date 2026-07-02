-- Derived from Luau 0.709 tests/conformance/pcall.luau C++ exception helper coverage.
local ok, err = pcall(cxxthrow)
return ok == false and err == "oops"
