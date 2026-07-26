-- Lua 5.4 equivalent of the intentional-retention positive control.
local ROWS = 128
local WIDTH = 32
local retained = {}
local step = 0

local function make_batch(seed)
  local batch = {}
  local checksum = 0
  for i = 1, ROWS do
    local row = {}
    for j = 1, WIDTH do
      row[j] = (seed * 131 + i * 17 + j * 7) % 65521
    end
    batch[i] = row
    checksum = checksum + row[1] + row[WIDTH]
  end
  return batch, checksum
end

return function(n)
  local checksum = 0
  for _ = 1, n do
    step = step + 1
    local batch, batch_checksum = make_batch(step)
    retained[#retained + 1] = batch
    checksum = checksum + batch_checksum
  end
  return checksum + step + #retained
end
