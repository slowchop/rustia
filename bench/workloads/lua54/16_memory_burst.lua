-- Lua 5.4 equivalent of memory_burst.
local ROWS = 512
local WIDTH = 64

return function(n)
  local checksum = 0
  for epoch = 1, n do
    local batch = {}
    for i = 1, ROWS do
      local row = {}
      for j = 1, WIDTH do
        row[j] = (epoch * 131 + i * 17 + j * 7) % 65521
      end
      batch[i] = row
      checksum = checksum + row[1] + row[WIDTH]
    end
    checksum = checksum + #batch
  end
  return checksum
end
