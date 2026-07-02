-- Workload: pcall / xpcall (protected calls, ~1 in 16 throws).
-- unit = one pcall + one xpcall pair; every 16th call raises and is caught.
-- Exercises: protected-call setup/teardown, error object creation+propagation, xpcall message handler.
local function maybeThrow(i)
  if i % 16 == 0 then error("boom") end
  return i * 2
end
local handler = function(_) return -1 end
local function f(n)
  local acc = 0
  for i = 1, n do
    local ok, v = pcall(maybeThrow, i)
    if ok then acc = acc + v end
    local ok2 = xpcall(maybeThrow, handler, i)
    if ok2 then acc = acc + 1 end
  end
  return acc
end
return f
