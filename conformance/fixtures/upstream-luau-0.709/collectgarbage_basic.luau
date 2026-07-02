-- Derived from the Luau 0.709 conformance harness collectgarbage helper.
assert(type(collectgarbage()) == "number")
assert(type(collectgarbage("collect")) == "number")
assert(type(collectgarbage("count")) == "number")
assert(not pcall(collectgarbage, "invalid-option"))

return "OK"
