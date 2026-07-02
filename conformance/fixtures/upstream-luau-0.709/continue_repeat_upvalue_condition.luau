-- Derived from Luau 0.709 tests/conformance/constructs.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers repeat/until continue preserving locals captured by closures used by the until condition.
local function check_connected(writer, reader)
    writer(1)
    assert(reader() == 1)
    return true
end

repeat
    local value = nil
    local function write(n)
        value = n
    end
    continue
until check_connected(write, function() return value end)

return 1
