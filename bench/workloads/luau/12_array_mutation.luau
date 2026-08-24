-- Workload: in-place dense-array mutation.
-- unit = one read-modify-write pass over a persistent 256-element array.
-- Exercises: integer-indexed stores into slots that already hold a value, with
-- no array growth, rehash, or length change -- the shape of particle, grid, and
-- vector update loops. Complements dense_tables, whose every store appends.
local S = 256
local function f(n)
  local t = {}
  for i = 1, S do t[i] = i end
  local checksum = 0.0
  for it = 1, n do
    for i = 1, S do
      t[i] = (t[i] * 3 + it) % 1000
    end
    checksum = checksum + t[1] + t[S]
  end
  return checksum
end
return f
