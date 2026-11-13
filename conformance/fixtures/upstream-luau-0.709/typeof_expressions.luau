-- Luau 0.709 tests/Parser.test.cpp: type_alias_to_a_typeof, with erased-execution controls.
-- Copyright (c) 2019-2025 Roblox Corporation
-- Copyright (c) 1994-2019 Lua.org, PUC-Rio.
-- MIT License; see conformance/upstream/luau-0.709/LICENSE.txt.
-- Commit: b968ef742741bb2b703afc3b3c53f06608c87481

do
    type A = typeof(1)
end
local observed = 0
type T = typeof((function() observed = 1; return 7 end)())
type U = typeof(`value {error('never execute')}`)
type V = typeof(function(x:number):number return x end)
local value: typeof(`plain`) = 42 :: typeof(error('never execute'))
assert(value == 42 and observed == 0)
return "OK"
