-- Workload: coroutine resume/yield throughput.
-- unit = one resume->yield round trip through a coroutine.wrap generator.
-- Exercises: coroutine stack switch, yield value marshalling, resume dispatch.
local function producer()
  return coroutine.wrap(function()
    local i = 0
    while true do
      i = i + 1
      coroutine.yield(i * 2)
    end
  end)
end
local function f(n)
  local acc = 0
  local gen = producer()
  for i = 1, n do
    acc = acc + gen()
  end
  return acc
end
return f
