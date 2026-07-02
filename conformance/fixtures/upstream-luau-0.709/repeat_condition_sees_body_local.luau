-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers repeat/until conditions seeing locals declared in the repeat body.
repeat
    local a = 5
until a - 4 < 0 or a - 4 >= 0
return nil
