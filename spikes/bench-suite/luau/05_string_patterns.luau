-- Workload: string PATTERNS (find / match / gmatch / gsub).
-- unit = over one ~60-char line: gmatch all %d+ runs, match a leading word, gsub all digits.
-- Exercises: the Lua pattern engine (the part rustia must reimplement byte-for-byte), tonumber, substring alloc.
local line = "the quick brown fox 123 jumps over 456 the lazy dog 789 end"
local function f(n)
  local acc = 0
  for it = 1, n do
    for num in string.gmatch(line, "%d+") do
      acc = acc + tonumber(num)
    end
    local w = string.match(line, "(%a+)%s")
    if w then acc = acc + #w end
    local _, cnt = string.gsub(line, "%d", "#")
    acc = acc + cnt
  end
  return acc
end
return f
