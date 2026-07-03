-- Workload: generic iteration (Lua 5.4 __pairs, pairs, ipairs).
-- unit = one pass over a 32-elem __pairs collection + a 5-key hashmap (pairs) + 32-elem array (ipairs).
-- Exercises: __pairs metamethod dispatch, pairs/next over hash, ipairs path.
local Coll = {}
Coll.__index = Coll
function Coll.__pairs(self)
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
    for _, v in pairs(c) do acc = acc + v end
    for _, v in pairs(hashmap) do acc = acc + v end
    for _, v in ipairs(c.items) do acc = acc + v end
  end
  return acc
end
return f
