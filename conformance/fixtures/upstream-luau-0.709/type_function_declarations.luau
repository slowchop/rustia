-- Excerpts from tests/Parser.test.cpp at Luau 0.709, with runtime erasure checks.
-- Copyright (c) 2019-2025 Roblox Corporation
-- Copyright (c) 1994-2019 Lua.org, PUC-Rio.
-- MIT License; see conformance/upstream/luau-0.709/LICENSE.txt.

local before = observed

-- parse_user_defined_type_functions
type function foo()
    observed = 17
    return types.number
end
export type function bar()
    return types.string
end
assert(foo == nil and bar == nil and observed == before)

-- parse_nested_type_function
local v1 = 1
type function outer()
    local v2 = 2
    local function change()
        v2 += 1
        type function inner() end
        v2 += 2
    end
    local function changeAgain() v2 += 3 end
end
local function change() v1 += 1 end
change()
assert(v1 == 2)

-- A type function creates no runtime binding, even when a value has its name.
local existing = 42
type function existing<T>(existing: T, ...: T): T
    local function get() return existing end
    return get()
end
type Applied = existing<number>
assert(existing == 42 and type(existing) == "number")
return "OK"
