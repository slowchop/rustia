-- Derived from Luau 0.709 tests/conformance/iter.luau getfenv substitution checks.
local env = getfenv(1)

env.__rustia_getfenv_marker = 41
local result = __rustia_getfenv_marker
env.__rustia_getfenv_marker = nil

return result
