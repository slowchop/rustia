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
return "OK"
