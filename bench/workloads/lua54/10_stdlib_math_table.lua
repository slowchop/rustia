-- Workload: stdlib hot paths (math + table).
-- unit = sin/cos/sqrt/abs/floor on a running value + push into a buffer; every 64 pushes, table.sort + flush.
-- Exercises: math builtin calls, table append, periodic table.sort.
local sin, cos, sqrt, floor, abs = math.sin, math.cos, math.sqrt, math.floor, math.abs
local function f(n)
  local acc = 0.0
  local t = {}
  local len = 0
  for i = 1, n do
    local x = i * 0.001
    acc = acc + sin(x) * cos(x) + sqrt(abs(x))
    len = len + 1
    t[len] = floor(acc) % 997
    if len >= 64 then
      table.sort(t)
      acc = acc + t[1]
      t = {}
      len = 0
    end
  end
  return acc
end
return f
