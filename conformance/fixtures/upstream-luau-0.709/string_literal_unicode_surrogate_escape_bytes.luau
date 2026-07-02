-- Derived from Luau 0.709 tests/conformance/utf8.luau surrogate escape cases.
return "\u{D800}\u{DFFF}" == "\237\160\128\237\191\191"
