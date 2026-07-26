-- Memory detector: a short, fully touched allocation burst that becomes unreachable before return.
-- The supervisor's cgroup peak must observe this even when periodic smaps sampling misses it.
local ROWS = 512
local WIDTH = 64

local function f(n)
  local checksum = 0
  for epoch = 1, n do
    local batch = {}
    for i = 1, ROWS do
      local row = {}
      for j = 1, WIDTH do
        row[j] = (epoch * 131 + i * 17 + j * 7) % 65521
      end
      batch[i] = row
      checksum += row[1] + row[WIDTH]
    end
    checksum += #batch
  end
  return checksum
end

return f
