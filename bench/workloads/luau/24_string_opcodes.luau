-- Workload: all nine dedicated string-library operations.
-- unit = one pass through sub, len, byte, upper, lower, reverse, rep, find, and split.
local source = "Rustia:StringOps/Alpha-Beta-Gamma-Delta-0123456789"

local n = BENCH_N
local checksum = 0
for i = 1, n do
  local start = (i % (#source - 8)) + 1
  local part = (string.sub(source, start, start + 7))
  local length = (string.len(part))
  local byte = (string.byte(source, start))
  local upper = (string.upper(part))
  local lower = (string.lower(upper))
  local reverse = (string.reverse(lower))
  local repeated_part = (string.sub(reverse, 1, 2))
  local repeated = (string.rep(repeated_part, 2 + (i % 3)))
  local find_part = (string.sub(source, start, start + 1))
  local found = (string.find(source, find_part, 1, true))
  local pieces = (string.split(source, ":"))
  checksum = checksum + length + byte + #upper + #lower + #reverse + #repeated
  checksum = checksum + (found or 0) + #pieces + #pieces[1] + i % 17
end

return checksum
