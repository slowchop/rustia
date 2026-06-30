-- Workload: closures & upvalues.
-- unit = one call to a captured closure that mutates a shared upvalue.
-- Exercises: closure allocation (setup), upvalue read/write through the closure, indirect call dispatch.
local function makeCounter(start, step)
  local c = start
  return function()
    c = c + step
    return c
  end
end
local function f(n)
  local acc = 0
  local counters = {}
  for i = 1, 8 do counters[i] = makeCounter(i, i) end
  for i = 1, n do
    local c = counters[(i % 8) + 1]
    acc = acc + c()
  end
  return acc
end
return f
