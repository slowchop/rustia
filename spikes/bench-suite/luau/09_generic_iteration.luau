-- Workload: generic iteration (Luau __iter generalized for, pairs, ipairs).
-- unit = one pass over a 32-elem __iter collection + a 5-key hashmap (pairs) + 32-elem array (ipairs).
-- Exercises: __iter metamethod dispatch (Luau-specific), pairs/next over hash, ipairs fast path.
local Coll = {}
Coll.__index = Coll
function Coll.__iter(self)
  local i = 0
  local items = self.items
  return function()
    i = i + 1
    if i <= #items then return i, items[i] end
  end
end
local function make(k)
  local items = {}
  for i = 1, k do items[i] = i * i end
  return setmetatable({ items = items }, Coll)
end
local hashmap = { a = 1, b = 2, c = 3, d = 4, e = 5 }
local function f(n)
  local acc = 0
  local c = make(32)
  for _ = 1, n do
    for _, v in c do acc = acc + v end
    for _, v in pairs(hashmap) do acc = acc + v end
    for _, v in ipairs(c.items) do acc = acc + v end
  end
  return acc
end
return f
