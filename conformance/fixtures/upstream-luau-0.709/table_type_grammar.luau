-- Source excerpts from Luau 0.709 tests/Parser.test.cpp, scoped for execution.
-- Copyright (c) 2019-2025 Roblox Corporation
-- Copyright (c) 1994-2019 Lua.org, PUC-Rio.
-- MIT License; see conformance/upstream/luau-0.709/LICENSE.txt.
-- Commit: b968ef742741bb2b703afc3b3c53f06608c87481
-- AST/typechecker expectations are not ported; this checks parsing and runtime erasure.

-- annotations_can_be_tables
do
    local zero: number
    local one: {x: number, y: string}
end

-- tables_should_have_an_indexer_and_keys
do
    local t: {
        [string]: number,
        f: () -> nil
    }
end

-- tables_can_have_trailing_separator
do
    local zero: number
    local one: {x: number, y: string, }
end

-- tables_can_use_semicolons
do
    local zero: number
    local one: {x: number; y: string; }
end

-- short_array_types
do
    local n: {string}
end

-- short_array_types_do_not_break_field_names
do
    local n: {string: number}
end

-- read_write_table_properties
do
    type A = {read x: number}
    type B = {write x: number}
    type C = {read x: number, write x: number}
    type D = {read: () -> string}
    type E = {write: (string) -> ()}
    type F = {read read: () -> string}
    type G = {read write: (string) -> ()}

    type H = {read ["A"]: number}
    type I = {write ["A"]: string}

    type J = {read [number]: number}
    type K = {write [number]: string}
end

-- parsing_string_union_indexers
do
    type foo = { ["bar" | "baz"]: number }
end

local value = {id = function(x) return x end} :: {id: <T>(T)->T}
assert(value.id(7) == 7)
return "OK"
