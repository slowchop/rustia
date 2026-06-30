-- Workload: string.format.
-- unit = one format call with mixed specifiers (%d %.3f %s %x), then measure result length.
-- Exercises: format-spec parsing, number->string (float + hex), concatenation into a fresh string.
local fmt = string.format
local function f(n)
  local acc = 0
  for i = 1, n do
    local s = fmt("[%d] x=%.3f name=%s hex=%x", i, i * 1.5, "agent", i)
    acc = acc + #s
  end
  return acc
end
return f
