-- Luau 0.709 test source excerpts, scoped and extended with runtime checks.
-- Copyright (c) 2019-2025 Roblox Corporation
-- Copyright (c) 1994-2019 Lua.org, PUC-Rio.
-- MIT License; see conformance/upstream/luau-0.709/LICENSE.txt.
-- Commit: b968ef742741bb2b703afc3b3c53f06608c87481
-- Typechecker expectations are not ported: this fixture checks syntax and erased execution.

-- TypeInfer.generics.test.cpp: local_vars_can_be_polytypes
do
    local function id<a>(x:a):a return x end
    local f: <a>(a)->a = id
    local x: string = f("hi")
    local y: number = f(37)
    assert(x == "hi" and y == 37)
end

-- TypeInfer.generics.test.cpp: function_arguments_can_be_polytypes
do
    local function f(g: <a>(a)->a)
        local x: number = g(37)
        local y: string = g("hi")
    end
    f(function(value) return value end)
end

-- TypeInfer.generics.test.cpp: function_results_can_be_polytypes
do
    local function f() : <a>(a)->a
        local function id<a>(x:a):a return x end
        return id
    end
    assert(f()(37) == 37)
end

-- TypeInfer.generics.test.cpp: type_parameters_can_be_polytypes
do
    local function id<a>(x:a):a return x end
    local f: <a>(a)->a = id(id)
    assert(f("hi") == "hi")
end

-- TypeInfer.generics.test.cpp: generic_functions_in_types
do
    type T = { id: <a>(a) -> a }
    local x: T = { id = function<a>(x:a):a return x end }
    local y: string = x.id("hi")
    local z: number = x.id(37)
    assert(y == "hi" and z == 37)
end

-- TypeInfer.generics.test.cpp: check_generic_typepack_function
do
    function id<a...>(...: a...): (a...) return ... end
    local x: string, y: boolean = id("hi", true)
    local z: number = id(37)
    id()
    assert(x == "hi" and y == true and z == 37)
end

-- Parser.test.cpp: two_left_and_right_arrows_but_no_explicit_type_instantiation
do
    type A = C<B<<T>() -> T>>
end

-- Parser.test.cpp: function_type_named_arguments (the four valid source snippets).
do
    type MyFunc = (a: number, b: string, c: number) -> string
    type MyFunc = (a: number, string, c: number) -> string
    type MyFunc = (a: number, string, number) -> string
    type MyFunc = (a: number, b: string, c: number) -> (d: number, e: string, f: number) -> string
end

return "OK"
