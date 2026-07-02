-- Workload: serialization round-trip (table -> string -> table).
-- unit = serialize a fixed 32-record array to a compact string, then parse it back into tables.
-- Exercises: table.concat string building, tostring/format, cursor-based parsing (string.find/sub + tonumber),
--            and table allocation on the rebuild. (Hand-rolled; no loadstring dependence.)
local function serialize(arr)
  local parts = {}
  local p = 0
  p = p + 1; parts[p] = "["
  for i = 1, #arr do
    local r = arr[i]
    p = p + 1; parts[p] = "{"
    p = p + 1; parts[p] = tostring(r.id)
    p = p + 1; parts[p] = ","
    p = p + 1; parts[p] = string.format("%.4f", r.x)
    p = p + 1; parts[p] = ","
    p = p + 1; parts[p] = r.name
    p = p + 1; parts[p] = "}"
    if i < #arr then p = p + 1; parts[p] = ";" end
  end
  p = p + 1; parts[p] = "]"
  return table.concat(parts)
end
local sub, find, tonum = string.sub, string.find, tonumber
local function parse(s)
  local arr = {}
  local len = #s
  local pos = 2 -- skip '['
  while pos <= len and sub(s, pos, pos) == "{" do
    pos = pos + 1
    local c1 = find(s, ",", pos, true)
    local id = tonum(sub(s, pos, c1 - 1))
    pos = c1 + 1
    local c2 = find(s, ",", pos, true)
    local x = tonum(sub(s, pos, c2 - 1))
    pos = c2 + 1
    local c3 = find(s, "}", pos, true)
    local name = sub(s, pos, c3 - 1)
    pos = c3 + 1
    arr[#arr + 1] = { id = id, x = x, name = name }
    if sub(s, pos, pos) == ";" then pos = pos + 1 end
  end
  return arr
end
local function f(n)
  local arr = {}
  for i = 1, 32 do arr[i] = { id = i, x = i * 1.25, name = "n" .. i } end
  local acc = 0
  for _ = 1, n do
    local s = serialize(arr)
    local back = parse(s)
    acc = acc + #s + #back + back[1].id + back[32].id
  end
  return acc
end
return f
