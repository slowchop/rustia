-- Workload: DENSE (array-part) tables.
-- unit = build a 256-element array, sum it, table.sort it, read endpoints.
-- Exercises: array-part growth, integer-indexed store/load, table.sort (comparison + swaps).
local S = 256
local function f(n)
  local checksum = 0.0
  for it = 1, n do
    local t = {}
    for i = 1, S do t[i] = (i * 2654435761 + it) % 1000 end
    local s = 0
    for i = 1, S do s = s + t[i] end
    table.sort(t)
    checksum = checksum + t[1] + t[S] + s
  end
  return checksum
end
return f
