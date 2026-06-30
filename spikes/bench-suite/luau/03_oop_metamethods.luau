-- Workload: metamethod-heavy OOP (the AI/gameplay shape).
-- unit = one physics-style step: (p + v) * scalar  + two method calls (:dot, :len2).
-- Exercises: __index method lookup, __add / __mul metamethods, setmetatable allocation per op, self-calls.
local Vec = {}
Vec.__index = Vec
function Vec.new(x, y) return setmetatable({ x = x, y = y }, Vec) end
Vec.__add = function(a, b) return Vec.new(a.x + b.x, a.y + b.y) end
Vec.__mul = function(a, s) return Vec.new(a.x * s, a.y * s) end
function Vec:dot(o) return self.x * o.x + self.y * o.y end
function Vec:len2() return self.x * self.x + self.y * self.y end
local function f(n)
  local acc = 0.0
  local p = Vec.new(1.0, 2.0)
  local v = Vec.new(0.5, -0.25)
  for i = 1, n do
    p = (p + v) * 0.999
    acc = acc + p:dot(v) + p:len2()
    if p.x > 1e6 or p.x < -1e6 then p = Vec.new(1.0, 2.0) end
  end
  return acc
end
return f
