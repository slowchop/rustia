-- Focused Luau 0.709 oracle regression for issue #19.
export type A = number
export type Box<T> = {value: T}
local export = 3
local x = (5 :: A)
local box = ({value=x} :: Box<A>)
assert(box.value == 5 and export == 3)
assert(1 + 2 :: number == 3)
assert(-5 :: number == -5)
local function values() return 1, 2 end
assert(select('#', values() :: any) == 1)
assert(select('#', values()) == 2)
assert((values() :: any) == 1)
local calls = 0
local function once() calls += 1; return {value=7} end
assert((once() :: {value: number}).value == 7)
assert(calls == 1)
local f = (function(a) return a end) :: (number) -> number
assert(f(9) == 9)
export type Imported = Module.Type
export type ImportedBox<T> = Module.Box<T>
assert((5 :: Module.Type) == 5)
assert((5 :: Module.type) == 5)
assert((5 :: Imported) == 5)
assert((box :: Module.Box<number>).value == 5)
-- Alias defaults are parsed and erased; packs remain distinct from single types.
export type DefaultBox<T = number> = { value: T }
type DefaultPair<T = number, U = string> = { first: T, second: U }
type DefaultCallback<T = () -> (number, string)> = T
type EmptyPack<P... = ()> = () -> P...
type SinglePack<P... = (number)> = () -> P...
type MultiplePack<T, P... = (number, string)> = (T) -> P...
type VariadicPack<P... = ...number?> = () -> P...
type NamedPack<P... = Other...> = () -> P...
type TailPack<P... = (number, ...string)> = () -> P...
type ObservedDefault<T = typeof((function() calls += 100; return 1 end)())> = T
local defaultBox = ({value = 9} :: DefaultBox)
assert(defaultBox.value == 9 and calls == 1)
assert((5 :: (number)) == 5 and (5 :: ((number))) == 5)
assert(select('#', values() :: (number)) == 1)
local multi = values :: () -> (number, number)
assert(select('#', multi()) == 2)
return "OK"
