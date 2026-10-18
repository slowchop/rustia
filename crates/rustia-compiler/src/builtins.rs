//! Builtin recognition from Luau 0.709 Compiler/src/Builtins.cpp.
//! This list selects call evaluation order; it does not grant an optimization.

pub(crate) fn global(name: &str) -> bool {
    matches!(
        name,
        "assert"
            | "type"
            | "typeof"
            | "rawset"
            | "rawget"
            | "rawequal"
            | "rawlen"
            | "unpack"
            | "getmetatable"
            | "setmetatable"
            | "tonumber"
            | "tostring"
    )
}

pub(crate) fn field(table: &str, name: &str) -> bool {
    match table {
        "math" => matches!(
            name,
            "abs"
                | "acos"
                | "asin"
                | "atan2"
                | "atan"
                | "ceil"
                | "cosh"
                | "cos"
                | "deg"
                | "exp"
                | "floor"
                | "fmod"
                | "frexp"
                | "ldexp"
                | "log10"
                | "log"
                | "max"
                | "min"
                | "modf"
                | "pow"
                | "rad"
                | "sinh"
                | "sin"
                | "sqrt"
                | "tanh"
                | "tan"
                | "clamp"
                | "sign"
                | "round"
                | "lerp"
                | "isnan"
                | "isinf"
                | "isfinite"
        ),
        "bit32" => matches!(
            name,
            "arshift"
                | "band"
                | "bnot"
                | "bor"
                | "bxor"
                | "btest"
                | "extract"
                | "lrotate"
                | "lshift"
                | "replace"
                | "rrotate"
                | "rshift"
                | "countlz"
                | "countrz"
                | "byteswap"
        ),
        "string" => matches!(name, "byte" | "char" | "len" | "sub"),
        "table" => matches!(name, "insert" | "unpack"),
        "buffer" => matches!(
            name,
            "readi8"
                | "readu8"
                | "writei8"
                | "writeu8"
                | "readi16"
                | "readu16"
                | "writei16"
                | "writeu16"
                | "readi32"
                | "readu32"
                | "writei32"
                | "writeu32"
                | "readf32"
                | "writef32"
                | "readf64"
                | "writef64"
        ),
        "vector" => matches!(
            name,
            "create"
                | "magnitude"
                | "normalize"
                | "cross"
                | "dot"
                | "floor"
                | "ceil"
                | "abs"
                | "sign"
                | "clamp"
                | "min"
                | "max"
                | "lerp"
        ),
        _ => false,
    }
}
