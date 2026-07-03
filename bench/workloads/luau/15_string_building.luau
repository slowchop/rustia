-- Workload: string building and concatenation.
-- unit = build 64 mixed string/number fragments, concatenate them, and measure the result length.
-- Exercises: numeric-to-string coercion through concat, table-backed buffering, and table.concat.
local fragments = { "alpha", "beta", "gamma", "delta" }
local fragment_count = #fragments

local function f(n)
  local checksum = 0
  for it = 0, n - 1 do
    local parts = {}
    for i = 0, 63 do
      local fragment = fragments[((i + it) % fragment_count) + 1]
      parts[i + 1] = fragment .. ":" .. (i % 17) .. ":" .. (it % 31) .. "|"
    end
    local s = table.concat(parts)
    checksum = checksum + #s
  end
  return checksum
end

return f
