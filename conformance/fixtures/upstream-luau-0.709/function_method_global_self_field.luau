-- Derived from Luau 0.709 tests/conformance/calls.luau method declaration using a global self table.
a = {i = 10}
self = 20

function a:x(x)
    return x + self.i
end

function a.y(x)
    return x + self
end

return a:x(1) + 10 == a.y(1)
