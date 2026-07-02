-- Derived from Luau 0.709 tests/conformance/calls.luau chained method calls returning self.
local a = {x = 0}

function a:add(x)
    self.x, a.y = self.x + x, 20
    return self
end

return a:add(10):add(20):add(30).x
