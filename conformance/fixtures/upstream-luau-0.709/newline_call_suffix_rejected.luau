-- Luau 0.709 tests/conformance/errors.luau and tests/Parser.test.cpp
-- (parse_error_confusing_function_call). License and revision in PROVENANCE.tsv.
assert(loadstring("a=math.sin\n(3)") == nil)
assert(loadstring("local f = print\n(f :: any)['x'] = 2") == nil)

local function identity(x) return x end
assert(identity
    "quoted" == "quoted")
assert(identity
    [[long]] == "long")
assert(identity
    {7}[1] == 7)

local obj = {get = function(self, value) return value end}
assert(obj:get -- a comment between the method and its argument
    "method" == "method")
assert(obj:get
    {value = 8}.value == 8)

-- A semicolon separates two statements when the second starts with '('.
local seen
local save = function(x) seen = x end;
(save)(9)
assert(seen == 9)
local number = 10
(save)(number)
assert(seen == 10)

-- The pinned lexer counts LF for call ambiguity. A lone CR is whitespace.
assert(loadstring("return math.abs\r(-3)")() == 3)
return "OK"
