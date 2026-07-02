local function checkerror(message, f, ...)
    local ok, err = pcall(f, ...)
    assert(not ok and string.find(err, message, 1, true))
end

checkerror("invalid UTF-8 code", utf8.codepoint, "\x80")
checkerror("invalid UTF-8 code", function()
    for _p, c in utf8.codes("ab\xff") do
        assert(c)
    end
end)
checkerror("position out of range", utf8.offset, "abc", 1, 5)
checkerror("continuation byte", utf8.offset, "\xF0\xA6\xA7\xBA", 1, 2)
checkerror("out of string", utf8.len, "abc", 0, 2)
checkerror("out of string", utf8.len, "abc", 1, 4)
checkerror("value out of range", utf8.char, 2147483648)
checkerror("value out of range", utf8.char, -1)
assert(pcall(utf8.codepoint, utf8.char(0xD800)) == false)
return "OK"
