-- Workload: SPARSE (hash-part) tables.
-- unit = fill a fresh table with 64 string keys + 64 scattered integer keys, then read them all back.
-- Exercises: hash-part insert/resize, string-key interning (the ".."), scattered-int hashing, miss/hit lookups.
local function f(n)
  local checksum = 0
  for it = 1, n do
    local t = {}
    for i = 1, 64 do
      t["k" .. i] = i * it
      t[(i * 7919) % 100003] = i
    end
    local s = 0
    for i = 1, 64 do
      s = s + (t["k" .. i] or 0) + (t[(i * 7919) % 100003] or 0)
    end
    checksum = checksum + s
  end
  return checksum
end
return f
