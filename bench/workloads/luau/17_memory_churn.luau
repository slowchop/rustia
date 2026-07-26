-- Memory detector null control: allocation continues, but the live graph is bounded to eight batches.
-- A reclaiming runtime may retain reusable capacity, but its tail growth should converge to a plateau.
local BATCHES = 8
local ROWS = 128
local WIDTH = 32
local ring = {}
local step = 0

local function makeBatch(seed)
  local batch = {}
  local checksum = 0
  for i = 1, ROWS do
    local row = {}
    for j = 1, WIDTH do
      row[j] = (seed * 131 + i * 17 + j * 7) % 65521
    end
    batch[i] = row
    checksum += row[1] + row[WIDTH]
  end
  return batch, checksum
end

local function f(n)
  local checksum = 0
  for _ = 1, n do
    step += 1
    local batch, batchChecksum = makeBatch(step)
    ring[((step - 1) % BATCHES) + 1] = batch
    checksum += batchChecksum
  end
  return checksum + step + #ring
end

return f
