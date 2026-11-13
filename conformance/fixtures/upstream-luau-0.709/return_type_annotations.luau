-- Source excerpts from Luau 0.709 tests/Parser.test.cpp, scoped for execution.
-- Copyright (c) 2019-2025 Roblox Corporation
-- Copyright (c) 1994-2019 Lua.org, PUC-Rio.
-- MIT License; see conformance/upstream/luau-0.709/LICENSE.txt.
-- Commit: b968ef742741bb2b703afc3b3c53f06608c87481
-- AST/typechecker expectations are not ported; this checks parsing and runtime erasure.

-- functions_can_have_return_annotations
do
    function foo(): number return 55 end
end

-- functions_can_have_a_function_type_annotation
do
    function f(): (number) -> nil return nil end
end

-- function_return_type_should_disambiguate_from_function_type_and_multiple_returns
do
    function f(): (number, string) return 1, "foo" end
end

-- function_return_type_should_parse_as_function_type_annotation_with_no_args
do
    function f(): () -> nil return nil end
end

-- parsing_type_suffix_for_return_type_with_variadic
do
    function foo(): (string, ...number) | boolean
    end
end

local function erased(): (number)? return 7 end
assert(erased() == 7)
return "OK"
