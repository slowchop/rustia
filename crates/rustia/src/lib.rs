#![forbid(unsafe_code)]

mod extension_api;
mod function;
mod macros;
mod serde_value;
mod value;

use function::IntoHostFunction;
use rustia_compiler::{compile_with_options, CompileError, CompileOptions};
use rustia_parser::SourceError;
use rustia_vm::{Program, Vm};
use std::fmt;

pub use extension_api::{
    RustiaBuilder, RustiaExtension, RustiaHostFuelPolicy, RustiaHostFunctionInfo,
    RUSTIA_DEFAULT_HOST_FUNCTION_FUEL, RUSTIA_HOST_FUEL_ABI_VERSION, RUSTIA_HOST_FUEL_POLICY,
    RUSTIA_MIN_HOST_FUNCTION_FUEL,
};
pub use function::{Exports, Function};
pub use rustia_vm::{
    Error as VmError, ErrorKind as VmErrorKind, OpCapability, Profile, VmSettings,
};
pub use serde_value::SerdeValueError;
pub use value::{
    FromRustia, FromRustiaMulti, IntoRustia, IntoRustiaMulti, OwnedKey, OwnedTable, OwnedValue,
};

pub struct Rustia {
    vm: Vm,
    generation: u64,
}

impl Rustia {
    pub fn builder() -> RustiaBuilder {
        RustiaBuilder::new()
    }

    pub fn new(profile: Profile) -> Self {
        Self {
            vm: Vm::new(profile),
            generation: 0,
        }
    }

    pub fn with_settings(profile: Profile, settings: VmSettings) -> Self {
        Self {
            vm: Vm::with_settings(profile, settings),
            generation: 0,
        }
    }

    pub fn compile_and_run(&mut self, source: &str) -> Result<Exports> {
        let value = self.compile_and_run_raw(source)?;
        Ok(Exports::new(value, self.generation))
    }

    pub fn compile_and_run_named(
        &mut self,
        source_name: impl Into<String>,
        source: &str,
    ) -> Result<Exports> {
        let value = self.compile_and_run_raw_named(source_name, source)?;
        Ok(Exports::new(value, self.generation))
    }

    pub fn compile_and_run_exports(&mut self, source: &str) -> Result<Exports> {
        self.compile_and_run(source)
    }

    pub fn compile_and_run_exports_named(
        &mut self,
        source_name: impl Into<String>,
        source: &str,
    ) -> Result<Exports> {
        self.compile_and_run_named(source_name, source)
    }

    pub fn run_owned(&mut self, source: &str) -> Result<OwnedValue> {
        let value = self.compile_and_run_raw(source)?;
        value::snapshot_value(&mut self.vm, value)
    }

    pub fn run_owned_named(
        &mut self,
        source_name: impl Into<String>,
        source: &str,
    ) -> Result<OwnedValue> {
        let value = self.compile_and_run_raw_named(source_name, source)?;
        value::snapshot_value(&mut self.vm, value)
    }

    pub fn run<T>(&mut self, source: &str) -> Result<T>
    where
        T: FromRustia,
    {
        let value = self.run_owned(source)?;
        T::from_rustia(value)
    }

    pub fn run_named<T>(&mut self, source_name: impl Into<String>, source: &str) -> Result<T>
    where
        T: FromRustia,
    {
        let value = self.run_owned_named(source_name, source)?;
        T::from_rustia(value)
    }

    pub(crate) fn compile_and_run_raw(&mut self, source: &str) -> Result<rustia_vm::Value> {
        let chunk = rustia_parser::parse_source(source).map_err(Error::Source)?;
        let program = rustia_compiler::compile(&chunk).map_err(Error::Compile)?;
        let value = self.vm.run(&program).map_err(Error::Vm)?;
        self.advance_generation();
        Ok(value)
    }

    pub(crate) fn compile_and_run_raw_named(
        &mut self,
        source_name: impl Into<String>,
        source: &str,
    ) -> Result<rustia_vm::Value> {
        let module = Self::compile(source_name, source)?;
        let value = self.vm.run(&module.program).map_err(Error::Vm)?;
        self.advance_generation();
        Ok(value)
    }

    pub fn insert_global<T>(&mut self, name: impl AsRef<str>, value: T) -> Result<()>
    where
        T: IntoRustia,
    {
        let value = value.into_rustia();
        let value = value::load_owned_value(&mut self.vm, &value)?;
        self.vm.set_global_value(name, value).map_err(Error::Vm)
    }

    pub fn insert_serde<T>(&mut self, name: impl AsRef<str>, value: &T) -> Result<()>
    where
        T: serde::Serialize + ?Sized,
    {
        let value = OwnedValue::from_serialize(value)?;
        let value = value::load_owned_value(&mut self.vm, &value)?;
        self.vm.set_global_value(name, value).map_err(Error::Vm)
    }

    pub fn add_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::Pure, function)
    }

    pub fn add_pure_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::Pure, function)
    }

    pub fn add_input_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::ReadsDeterministicInput, function)
    }

    pub fn add_slot_read_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::ReadsSlotState, function)
    }

    pub fn add_slot_write_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::WritesSlotEffect, function)
    }

    pub fn add_effect_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::EmitsDeterministicEffect, function)
    }

    pub fn add_external_state_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::ReadsExternalMutableState, function)
    }

    pub fn add_nondeterministic_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::NondeterministicEffect, function)
    }

    pub fn add_fn_with<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        capability: OpCapability,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        let op = function.into_host_function(name.into(), fuel_cost, capability);
        self.vm.install_host_function(op).map_err(Error::Vm)
    }

    pub fn install_extension(&mut self, extension: RustiaExtension) -> Result<()> {
        extension.install(self)
    }

    pub fn compile(source_name: impl Into<String>, source: &str) -> Result<CompiledModule> {
        let source_name = source_name.into();
        let chunk = rustia_parser::parse_source(source).map_err(Error::Source)?;
        let options = CompileOptions::default().with_source(source_name.clone(), source);
        let program = compile_with_options(&chunk, options).map_err(Error::Compile)?;
        Ok(CompiledModule {
            source_name,
            program,
        })
    }

    pub fn install(&mut self, module: CompiledModule) -> Result<Exports> {
        let value = self.vm.run(&module.program).map_err(Error::Vm)?;
        self.advance_generation();
        Ok(Exports::new(value, self.generation))
    }

    pub fn hot_reload(&mut self, source_name: impl Into<String>, source: &str) -> Result<Exports> {
        let module = Self::compile(source_name, source)?;
        self.install(module)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn advance_generation(&mut self) {
        self.generation = self.generation.saturating_add(1);
    }

    #[inline]
    pub(crate) fn vm_mut(&mut self) -> &mut Vm {
        &mut self.vm
    }
}

impl Default for Rustia {
    fn default() -> Self {
        Self::new(Profile::ReproducibleStandard)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct CompiledModule {
    source_name: String,
    program: Program,
}

impl CompiledModule {
    pub fn source_name(&self) -> &str {
        &self.source_name
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

impl From<rustia_lexer::Span> for SourceSpan {
    fn from(span: rustia_lexer::Span) -> Self {
        Self {
            start: span.start,
            end: span.end,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceLocation<'a> {
    pub source: &'a str,
    pub line: u32,
}

#[derive(Debug)]
pub enum Error {
    Source(SourceError),
    Compile(CompileError),
    Vm(VmError),
    Embed(String),
}

impl Error {
    pub fn source_span(&self) -> Option<SourceSpan> {
        match self {
            Self::Source(SourceError::Lex(error)) => Some(error.span.into()),
            Self::Source(SourceError::Parse(error)) => Some(error.span.into()),
            Self::Compile(error) => error.span.map(Into::into),
            Self::Vm(_) | Self::Embed(_) => None,
        }
    }

    pub fn source_location(&self) -> Option<SourceLocation<'_>> {
        match self {
            Self::Vm(error) => error.location.as_ref().map(|location| SourceLocation {
                source: &location.source,
                line: location.line,
            }),
            Self::Source(_) | Self::Compile(_) | Self::Embed(_) => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => error.fmt(formatter),
            Self::Compile(error) => error.fmt(formatter),
            Self::Vm(error) => error.fmt(formatter),
            Self::Embed(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::{
        Error, FromRustia, OpCapability, OwnedTable, OwnedValue, Profile, Result, Rustia,
        RustiaExtension, RustiaHostFunctionInfo, SourceSpan, VmErrorKind, VmSettings,
        RUSTIA_DEFAULT_HOST_FUNCTION_FUEL, RUSTIA_HOST_FUEL_ABI_VERSION, RUSTIA_HOST_FUEL_POLICY,
        RUSTIA_MIN_HOST_FUNCTION_FUEL,
    };
    use rustia_compiler::{compile_with_options, CompileOptions};
    use rustia_parser::parse_source;
    use rustia_vm::Vm;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use std::num::NonZeroU64;

    #[test]
    fn compile_and_run_smoke() {
        let mut rustia = Rustia::new(Profile::DeterministicSim);
        let result = rustia
            .run_owned("local x: number = 6 * 7; return x")
            .unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 42.0);
    }

    #[test]
    fn typed_run_converts_owned_results() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);

        let answer: f64 = rustia.run("return 40 + 2").unwrap();
        let numbers: Vec<i32> = rustia.run("return { 1, 2, 3 }").unwrap();
        let missing: Option<String> = rustia.run("return nil").unwrap();
        let named: BTreeMap<String, i32> = rustia.run("return { scout = 5, brute = 9 }").unwrap();

        assert_eq!(answer, 42.0);
        assert_eq!(numbers, vec![1, 2, 3]);
        assert_eq!(missing, None);
        assert_eq!(named.get("scout"), Some(&5));
        assert_eq!(named.get("brute"), Some(&9));
    }

    #[test]
    fn global_reads_track_writes_after_inline_cache_hits() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: f64 = rustia
            .run(
                "
counter = 0
local total = 0
for _ = 1, 4 do
    total = total + counter
    counter = counter + 1
end
return total
",
            )
            .unwrap();

        assert_eq!(value, 6.0);
    }

    #[test]
    fn standard_global_inline_cache_is_shadowed_by_assignment() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local function call_type()
    return type(1)
end

local before = call_type()
type = function()
    return "shadow"
end
local after = call_type()
return before .. "|" .. after
"#,
            )
            .unwrap();

        assert_eq!(value, "number|shadow");
    }

    #[test]
    fn table_hash_index_tracks_numeric_key_promotion_to_array() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: f64 = rustia
            .run(
                "
local t = {}
t[4] = 4
local before = t[4]
t[1] = 1
t[2] = 2
t[3] = 3
t[4] = 40
t[5] = 5
return before + t[4] + t[5]
",
            )
            .unwrap();

        assert_eq!(value, 49.0);
    }

    #[test]
    fn table_hash_index_tracks_delete_and_clear() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local t = { alpha = 1, beta = 2, gamma = 3 }
t.beta = nil
local deleted = tostring(t.beta)
table.clear(t)
t.alpha = 10
return deleted .. "|" .. tostring(t.alpha) .. "|" .. tostring(t.gamma)
"#,
            )
            .unwrap();

        assert_eq!(value, "nil|10|nil");
    }

    #[test]
    fn table_hash_index_tracks_many_hash_keys() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local t = {}
for i = 1, 24 do
    t["k" .. i] = i
    t[i * 1000] = i * 2
end
local before = t["k1"] + t["k24"] + t[24000]
t["k17"] = nil
t[2000] = nil
t[30] = 3000
local sparse = t[30]
for i = 1, 30 do
    t[i] = i
end
local promoted = t[30]
t[30] = 3030
return table.concat({
    before,
    tostring(t["k17"] == nil),
    tostring(t[2000] == nil),
    sparse,
    promoted,
    t[30],
    t["k23"],
    t[23000],
}, "|")
"#,
            )
            .unwrap();

        assert_eq!(value, "73|true|true|3000|30|3030|23|46");
    }

    #[test]
    fn numeric_fast_path_preserves_coercion_and_metamethod_fallbacks() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local mt = {
    __add = function(lhs, rhs)
        return lhs.value + rhs
    end
}
local boxed = setmetatable({ value = 5 }, mt)
return tostring("2" + 3) .. "|" .. tostring(boxed + 7)
"#,
            )
            .unwrap();

        assert_eq!(value, "5|12");
    }

    #[test]
    fn arithmetic_metamethod_inline_cache_tracks_left_changes_and_right_fallback() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local function add(lhs, rhs)
    return lhs + rhs
end

local mt1 = {
    __add = function(lhs, rhs)
        return lhs.value + rhs.value
    end
}
local mt2 = {
    __add = function(lhs, rhs)
        return lhs.value * rhs.value
    end
}
local rhs_mt = {
    __add = function()
        return 99
    end
}

local lhs = setmetatable({ value = 2 }, mt1)
local rhs = { value = 3 }
local first = add(lhs, rhs)
mt1.__add = function(lhs, rhs)
    return lhs.value - rhs.value
end
local second = add(lhs, rhs)
setmetatable(lhs, mt2)
local third = add(lhs, rhs)
mt2.__add = nil
setmetatable(rhs, rhs_mt)
local fourth = add(lhs, rhs)
return table.concat({ first, second, third, fourth }, "|")
"#,
            )
            .unwrap();

        assert_eq!(value, "5|-1|6|99");
    }

    #[test]
    fn setmetatable_native_fast_path_preserves_call_edges() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local t = setmetatable({}, { __index = { answer = 42 } })
local function none()
end
local ok_missing = pcall(function()
    setmetatable({}, none())
end)
local protected = setmetatable({}, { __metatable = "locked" })
local ok_protected, protected_error = pcall(function()
    setmetatable(protected, {})
end)
local local_setmetatable = function()
    return "shadowed"
end
local shadowed = local_setmetatable({}, {})
setmetatable = function()
    return "global"
end
local replaced = setmetatable({}, {})
return table.concat({
    t.answer,
    tostring(ok_missing),
    tostring(ok_protected),
    tostring(string.find(protected_error, "protected") ~= nil),
    shadowed,
    replaced,
}, "|")
"#,
            )
            .unwrap();

        assert_eq!(value, "42|false|false|true|shadowed|global");
    }

    #[test]
    fn concat_integer_fast_path_preserves_number_text_edges() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local a = "k" .. 1
local b = 12 .. "k"
local c = "n" .. 999999
local d = "z" .. 0
local e = "m" .. -0
local f = "d" .. 1.5
return table.concat({ a, b, c, d, e, f }, "|")
"#,
            )
            .unwrap();

        assert_eq!(value, "k1|12k|n999999|z0|m-0|d1.5");
    }

    #[test]
    fn native_string_len_fast_path_preserves_first_class_coercion() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: f64 = rustia
            .run(
                r#"
local len = string.len
local s = "abcdef"
return len("abc") + len(123) + #s
"#,
            )
            .unwrap();

        assert_eq!(value, 12.0);
    }

    #[test]
    fn string_sub_preserves_positive_negative_and_empty_ranges() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local sub = string.sub
return sub("abcdef", 2, 4) .. "|" .. sub("abcdef", -3, -1) .. "|" .. tostring(#sub("abc", 5, 6))
"#,
            )
            .unwrap();

        assert_eq!(value, "bcd|def|0");
    }

    #[test]
    fn string_format_plan_cache_preserves_reused_format_arguments() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local fmt = string.format
local pattern = "[%d] x=%.3f name=%s hex=%x %% %*"
local a = fmt(pattern, 7, 10.5, "agent", 255, true)
local b = fmt(pattern, 8, 12, "bee", 16, false)
return a .. "|" .. b
"#,
            )
            .unwrap();

        assert_eq!(
            value,
            "[7] x=10.500 name=agent hex=ff % true|[8] x=12.000 name=bee hex=10 % false"
        );
    }

    #[test]
    fn string_format_direct_path_preserves_simple_and_padded_specs() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local fmt = string.format
return table.concat({
    fmt("%d|%+d|% d", -42, 42, 42),
    fmt("%u|%o|%x|%X", -1, 511, 48879, 48879),
    fmt("%c|%s|%.3f|%+f|% f", 65, "ok", 1.5, 2.0, 2.0),
    fmt("%5d|%.4x|%8s", 7, 255, "hi"),
}, "|")
"#,
            )
            .unwrap();

        assert_eq!(
            value,
            "-42|+42| 42|18446744073709551615|777|beef|BEEF|A|ok|1.500|+2.000000| 2.000000|    7|00ff|      hi"
        );
    }

    #[test]
    fn table_index_fast_path_preserves_index_metamethod_fallbacks() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local mt = {
    __index = function(_, key)
        return "fallback:" .. key
    end
}
local t = setmetatable({ present = "raw", erased = "raw" }, mt)
t.erased = nil
return t.present .. "|" .. t.missing .. "|" .. t.erased
"#,
            )
            .unwrap();

        assert_eq!(value, "raw|fallback:missing|fallback:erased");
    }

    #[test]
    fn array_index_fast_path_preserves_sparse_keys_and_metamethods() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local sparse = {}
sparse[100000] = 7

local t = setmetatable({ 11, 22 }, {
    __index = function(_, key)
        return "fallback:" .. tostring(key)
    end,
    __newindex = function(target, key, value)
        rawset(target, "seen", tostring(key) .. ":" .. tostring(value))
    end
})

local before = t[1]
t[1] = 33
t[2] = nil
t[3] = 44

return tostring(sparse[100000])
    .. "|"
    .. tostring(before)
    .. "|"
    .. tostring(t[1])
    .. "|"
    .. tostring(t[2])
    .. "|"
    .. tostring(t[3])
    .. "|"
    .. tostring(t.seen)
"#,
            )
            .unwrap();

        assert_eq!(value, "7|11|33|fallback:2|fallback:3|3:44");
    }

    #[test]
    fn table_get_inline_cache_tracks_static_field_changes() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local t = { value = "a" }
local function read()
    return t.value
end

local a = read()
local b = read()
t.value = "b"
local c = read()
t.value = nil
local d = tostring(read())
return a .. b .. c .. d
"#,
            )
            .unwrap();

        assert_eq!(value, "aabnil");
    }

    #[test]
    fn table_get_inline_cache_tracks_metatable_after_nil_hit() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local t = {}
local function read()
    return t.missing
end

local before = tostring(read())
setmetatable(t, {
    __index = function(_, key)
        return "fallback:" .. key
    end
})
local after = read()
return before .. "|" .. after
"#,
            )
            .unwrap();

        assert_eq!(value, "nil|fallback:missing");
    }

    #[test]
    fn table_get_inline_cache_preserves_static_field_metamethod_calls() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local calls = 0
local t = setmetatable({}, {
    __index = function(_, key)
        calls = calls + 1
        return key .. ":" .. tostring(calls)
    end
})

local function read()
    return t.missing
end

return read() .. "|" .. read() .. "|" .. tostring(calls)
"#,
            )
            .unwrap();

        assert_eq!(value, "missing:1|missing:2|2");
    }

    #[test]
    fn table_get_inline_cache_tracks_index_table_changes_and_raw_overrides() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local proto = { value = "a" }
local mt = { __index = proto }
local first = setmetatable({}, mt)
local second = setmetatable({}, mt)

local function read(object)
    return object.value
end

local a = read(first)
local b = read(second)
second.value = "local"
local c = read(second)
proto.value = "b"
local d = read(first)
mt.__index = { value = "c" }
local e = read(first)
return table.concat({ a, b, c, d, e }, "|")
"#,
            )
            .unwrap();

        assert_eq!(value, "a|a|local|b|c");
    }

    #[test]
    fn len_inline_cache_tracks_table_changes_and_metamethods() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local t = { 1, 2, 3 }
local out = {}
for i = 1, 3 do
    out[i] = #t
    if i == 1 then
        t[4] = 4
    elseif i == 2 then
        t[4] = nil
    end
end

local calls = 0
local proxy = setmetatable({ 1, 2, 3 }, {
    __len = function()
        calls = calls + 1
        return 40 + calls
    end
})
return table.concat(out, ",") .. "|" .. #proxy .. "|" .. #proxy .. "|" .. calls
"#,
            )
            .unwrap();

        assert_eq!(value, "3,4,3|41|42|2");
    }

    #[test]
    fn iter_metamethod_inline_cache_tracks_metatable_changes() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local t = { 1, 2 }

local function walk()
    local out = {}
    for k, v in t do
        out[#out + 1] = tostring(k) .. ":" .. tostring(v)
    end
    return table.concat(out, ",")
end

local before = walk()
setmetatable(t, {
    __iter = function(self)
        local i = 0
        return function()
            i = i + 1
            if i <= 2 then
                return i, self[i] * 10
            end
        end
    end,
})
local installed = walk()
getmetatable(t).__iter = function()
    local done = false
    return function()
        if not done then
            done = true
            return "x", 99
        end
    end
end
local replaced = walk()
setmetatable(t, nil)
local removed = walk()
return before .. "|" .. installed .. "|" .. replaced .. "|" .. removed
"#,
            )
            .unwrap();

        assert_eq!(value, "1:1,2:2|1:10,2:20|x:99|1:1,2:2");
    }

    #[test]
    fn set_index_fast_path_preserves_newindex_metamethod_fallbacks() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local proxy = setmetatable({}, {
    __newindex = function(target, key, value)
        rawset(target, "seen", key .. ":" .. tostring(value))
    end
})
proxy.answer = 42
local direct = setmetatable({ answer = 1 }, {
    __newindex = function(target, key, value)
        rawset(target, "seen", key .. ":" .. tostring(value))
    end
})
direct.answer = 2
local plain = {}
plain.answer = 42
return proxy.seen .. "|" .. tostring(direct.answer) .. "|" .. tostring(direct.seen) .. "|" .. tostring(plain.answer)
"#,
            )
            .unwrap();

        assert_eq!(value, "answer:42|2|nil|42");
    }

    #[test]
    fn array_set_fast_path_preserves_newindex_and_readonly_edges() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local log = {}
local proxy = setmetatable({ "a" }, {
    __newindex = function(target, key, value)
        log[#log + 1] = tostring(key) .. ":" .. tostring(value)
        rawset(target, key, value)
    end
})
proxy[2] = "two"
proxy[1] = "one"
proxy[3] = nil
local frozen = table.freeze({ 1 })
local ok, message = pcall(function()
    frozen[1] = 2
end)
local readonly = string.find(message, "readonly", 1, true) ~= nil
local frozen_sparse = table.freeze({})
local sparse_ok, sparse_message = pcall(function()
    frozen_sparse[100000] = 2
end)
local sparse_readonly = string.find(sparse_message, "readonly", 1, true) ~= nil
return proxy[1]
    .. "|" .. proxy[2]
    .. "|" .. tostring(proxy[3])
    .. "|" .. table.concat(log, ",")
    .. "|" .. tostring(ok)
    .. "|" .. tostring(readonly)
    .. "|" .. tostring(sparse_ok)
    .. "|" .. tostring(sparse_readonly)
"#,
            )
            .unwrap();

        assert_eq!(value, "one|two|nil|2:two,3:nil|false|true|false|true");
    }

    #[test]
    fn table_sort_fast_path_preserves_numeric_and_comparator_results() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local a = { 9, 4, 7, 1, 4 }
table.sort(a)
local b = { 1, 4, 7, 9 }
table.sort(b, function(left, right)
    return left > right
end)
local c = { 0, -0 }
table.sort(c)
local d = { 5, -2, 5, -1 }
table.sort(d)
return table.concat(a, ",") .. "|" .. table.concat(b, ",") .. "|" .. tostring(c[1]) .. "," .. tostring(c[2]) .. "|" .. table.concat(d, ",")
"#,
            )
            .unwrap();

        assert_eq!(value, "1,4,4,7,9|9,7,4,1|0,-0|-2,-1,5,5");
    }

    #[test]
    fn aliased_math_fast_path_preserves_native_call_edges() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local sin, floor, abs = math.sin, math.floor, math.abs
local ok, message = pcall(function()
    return abs()
end)
local has_missing_abs = string.find(message, "missing argument #1 to 'abs'", 1, true) ~= nil
return tostring(sin("0", "ignored"))
    .. "|" .. tostring(floor(2.9, 99))
    .. "|" .. tostring(abs("-3", "ignored"))
    .. "|" .. tostring(ok)
    .. "|" .. tostring(has_missing_abs)
"#,
            )
            .unwrap();

        assert_eq!(value, "0|2|3|false|true");
    }

    #[test]
    fn upvalue_peephole_preserves_adjacent_reads_and_return_reload() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: String = rustia
            .run(
                r#"
local function make(start, step)
    local current = start
    return function()
        current = current + step
        return current
    end
end
local a = make(1, 2)
local b = make(10, -1)
return table.concat({ a(), a(), b(), a(), b() }, ",")
"#,
            )
            .unwrap();

        assert_eq!(value, "3,5,9,7,8");
    }

    #[test]
    fn collectgarbage_sees_current_frame_register_updates() {
        let settings = VmSettings {
            deterministic_gc_interval: NonZeroU64::new(u64::MAX).unwrap(),
            ..VmSettings::default()
        };
        let mut rustia = Rustia::with_settings(Profile::ReproducibleStandard, settings);
        let result = rustia
            .run_owned(
                "\
local weak = setmetatable({}, { __mode = 'v' })
weak[1] = (function()
    return {}
end)()
local strong = weak[1]
collectgarbage()
return strong ~= nil and weak[1] == strong
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn collectgarbage_observes_late_weak_mode_assignment() {
        let settings = VmSettings {
            deterministic_gc_interval: NonZeroU64::new(u64::MAX).unwrap(),
            ..VmSettings::default()
        };
        let mut rustia = Rustia::with_settings(Profile::ReproducibleStandard, settings);
        let result = rustia
            .run_owned(
                "\
local mt = {}
local weak = setmetatable({}, mt)
local function put()
    weak[1] = {}
end
put()
mt.__mode = 'v'
collectgarbage()
return weak[1] == nil
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn typed_run_reports_conversion_errors() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let error = rustia.run::<String>("return 42").unwrap_err().to_string();
        assert!(error.contains("expected string, got number"), "{error}");

        let array_error = rustia
            .run::<BTreeMap<String, i32>>("return { 1, 2 }")
            .unwrap_err()
            .to_string();
        assert!(
            array_error.contains("expected string-keyed table, got table with array entries"),
            "{array_error}"
        );

        let key_error = rustia
            .run::<BTreeMap<String, i32>>("return { [true] = 1 }")
            .unwrap_err()
            .to_string();
        assert!(
            key_error.contains("expected string-keyed table, got non-string key"),
            "{key_error}"
        );
    }

    #[test]
    fn numeric_table_keys_normalize_negative_zero_at_script_boundary() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .run_owned(
                "\
local t = {}
t[-0] = 11
t[0] = t[0] + 31
return t[-0] == 42 and t[0] == 42 and rawget(t, -0) == 42 and rawget(t, 0) == 42
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn fuel_exhaustion_escapes_pcall_and_xpcall() {
        let cases = [
            "return pcall(function() while true do end end)",
            "return xpcall(function() while true do end end, function() return 'caught' end)",
        ];

        for source in cases {
            let settings = VmSettings {
                initial_fuel: Some(1_000),
                ..VmSettings::default()
            };
            let mut rustia = Rustia::with_settings(Profile::DeterministicSim, settings);
            let Err(Error::Vm(error)) = rustia.run_owned(source) else {
                panic!("expected fuel exhaustion to escape protected call: {source}");
            };
            assert!(matches!(
                error.kind,
                VmErrorKind::FuelExhausted { limit: 1_000 }
            ));
        }
    }

    #[test]
    fn owned_values_snapshot_returned_tables() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value = rustia
            .run_owned(
                "\
return {
    label = 'spawn',
    count = 3,
    flags = { true, false },
}
",
            )
            .unwrap();
        let table = value.table().unwrap();
        assert_eq!(
            table.get_string("label"),
            Some(&OwnedValue::String("spawn".to_owned()))
        );
        assert_eq!(table.get_string("count"), Some(&OwnedValue::Number(3.0)));
        let flags = table.get_string("flags").unwrap().table().unwrap();
        assert_eq!(
            flags.array,
            vec![OwnedValue::Boolean(true), OwnedValue::Boolean(false)]
        );
    }

    #[test]
    fn owned_table_helpers_accept_typed_values_and_getters() {
        let labels = OwnedTable::string_entries([("kind", "scout"), ("role", "ai")]);
        assert_eq!(
            labels.get_string_as::<String>("kind").unwrap().as_deref(),
            Some("scout")
        );
        assert_eq!(labels.get_string_as::<String>("missing").unwrap(), None);

        let stats = OwnedTable::string_entries([("hp", 5_i32), ("speed", 7_i32)]);
        assert_eq!(stats.get_string_as::<i32>("hp").unwrap(), Some(5));

        let error = labels
            .get_string_as::<bool>("kind")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("expected boolean, got string"),
            "unexpected error: {error}"
        );

        let array = OwnedTable::array_values([1_i32, 2, 3]);
        assert_eq!(
            array.array,
            vec![
                OwnedValue::Number(1.0),
                OwnedValue::Number(2.0),
                OwnedValue::Number(3.0)
            ]
        );
        assert!(array.entries.is_empty());
    }

    #[test]
    fn owned_table_helpers_support_nested_typed_access() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value = rustia
            .run_owned(
                "\
return {
    config = {
        spawn = {
            kind = 'scout',
            stats = { 5, 7 },
        },
    },
}
",
            )
            .unwrap();

        let root = value.table().unwrap();
        let config = root.get_string_table("config").unwrap().unwrap();
        let spawn = config.get_string_table("spawn").unwrap().unwrap();
        assert_eq!(
            spawn.get_string_as::<String>("kind").unwrap().as_deref(),
            Some("scout")
        );
        assert!(spawn.get_string_table("missing").unwrap().is_none());

        let stats = spawn.get_string_table("stats").unwrap().unwrap();
        assert_eq!(stats.get_array(0), Some(&OwnedValue::Number(5.0)));
        assert_eq!(stats.get_array_as::<i32>(0).unwrap(), Some(5));
        assert_eq!(stats.get_array_as::<i32>(1).unwrap(), Some(7));
        assert_eq!(stats.get_array_as::<i32>(2).unwrap(), None);

        let string_error = spawn.get_string_table("kind").unwrap_err().to_string();
        assert!(
            string_error.contains("expected table, got string"),
            "unexpected error: {string_error}"
        );
        let array_error = stats.get_array_table(0).unwrap_err().to_string();
        assert!(
            array_error.contains("expected table, got number"),
            "unexpected error: {array_error}"
        );
    }

    #[test]
    fn owned_table_helpers_deserialize_nested_fields() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct SpawnConfig<'a> {
            kind: &'a str,
            hp: u32,
            stats: Vec<u32>,
        }

        #[derive(Debug, Deserialize, PartialEq)]
        struct Enemy<'a> {
            kind: &'a str,
            hp: u32,
        }

        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value = rustia
            .run_owned(
                "\
return {
    config = {
        kind = 'scout',
        hp = 5,
        stats = { 7, 9 },
    },
    enemies = {
        { kind = 'brute', hp = 9 },
        { kind = 'scout', hp = 5 },
    },
}
",
            )
            .unwrap();

        let root = value.table().unwrap();
        let config = root
            .get_string_deserialized::<SpawnConfig<'_>>("config")
            .unwrap()
            .unwrap();
        assert_eq!(
            config,
            SpawnConfig {
                kind: "scout",
                hp: 5,
                stats: vec![7, 9]
            }
        );
        assert!(root
            .get_string_deserialized::<SpawnConfig<'_>>("missing")
            .unwrap()
            .is_none());

        let enemies = root.get_string_table("enemies").unwrap().unwrap();
        assert_eq!(
            enemies.get_array_deserialized::<Enemy<'_>>(0).unwrap(),
            Some(Enemy {
                kind: "brute",
                hp: 9
            })
        );
        assert_eq!(
            enemies.get_array_deserialized::<Enemy<'_>>(1).unwrap(),
            Some(Enemy {
                kind: "scout",
                hp: 5
            })
        );
        assert!(enemies
            .get_array_deserialized::<Enemy<'_>>(2)
            .unwrap()
            .is_none());
    }

    #[test]
    fn insert_global_accepts_option_vec_and_owned_table() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let named_map = BTreeMap::from([("scout".to_owned(), 5_i32), ("brute".to_owned(), 9_i32)]);
        rustia.insert_global("maybe", Some(4_i32)).unwrap();
        rustia
            .insert_global("missing", Option::<i32>::None)
            .unwrap();
        rustia.insert_global("numbers", vec![1_i32, 2, 3]).unwrap();
        rustia.insert_global("named_map", named_map).unwrap();
        rustia
            .insert_global("named", OwnedTable::string_entries([("kind", "scout")]))
            .unwrap();

        let value = rustia
            .run_owned(
                "\
return maybe == 4
    and missing == nil
    and numbers[1] == 1
    and numbers[3] == 3
    and named_map.scout == 5
    and named_map.brute == 9
    and named.kind == 'scout'
",
            )
            .unwrap();
        assert!(bool::from_rustia(value).unwrap());
    }

    #[test]
    fn insert_global_and_typed_add_fn_are_callable() {
        let mut rustia = Rustia::new(Profile::DeterministicSim);
        rustia.insert_global("difficulty", 2_i32).unwrap();
        rustia
            .add_fn("clamp", |x: f64, lo: f64, hi: f64| x.clamp(lo, hi))
            .unwrap();

        let result = rustia
            .run_owned("return clamp(12 + difficulty, 0, 10)")
            .unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 10.0);
    }

    #[test]
    fn typed_host_functions_support_eight_arguments() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        rustia
            .add_fn(
                "sum8",
                |a: f64, b: f64, c: f64, d: f64, e: f64, f: f64, g: f64, h: f64| {
                    a + b + c + d + e + f + g + h
                },
            )
            .unwrap();

        let result = rustia
            .run_owned("return sum8(1, 2, 3, 4, 5, 6, 7, 8)")
            .unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 36.0);
    }

    #[test]
    fn typed_host_functions_accept_and_return_owned_rust_shapes() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        rustia
            .add_fn("sum_vec", |values: Vec<f64>| {
                values.into_iter().sum::<f64>()
            })
            .unwrap();
        rustia
            .add_fn("maybe_name", |enabled: bool| {
                enabled.then_some("scout".to_owned())
            })
            .unwrap();
        rustia.add_fn("make_vec", || vec![2.0, 4.0, 6.0]).unwrap();
        rustia
            .add_fn("sum_map", |values: BTreeMap<String, f64>| {
                values.values().sum::<f64>()
            })
            .unwrap();
        rustia
            .add_fn("make_map", || {
                BTreeMap::from([("scout".to_owned(), 5.0), ("brute".to_owned(), 9.0)])
            })
            .unwrap();
        rustia
            .add_fn("make_table", || {
                OwnedTable::string_entries([("kind", "scout")])
            })
            .unwrap();

        let value = rustia
            .run_owned(
                "\
return sum_vec({ 1, 2, 3 }) == 6
    and maybe_name(true) == 'scout'
    and maybe_name(false) == nil
    and make_vec()[2] == 4
    and sum_map({ scout = 5, brute = 9 }) == 14
    and make_map().brute == 9
    and make_table().kind == 'scout'
",
            )
            .unwrap();
        assert!(bool::from_rustia(value).unwrap());
    }

    #[test]
    fn facade_capability_helpers_register_allowed_deterministic_functions() {
        let mut rustia = Rustia::new(Profile::DeterministicSim);
        rustia.add_pure_fn("pure", || 5.0).unwrap();
        rustia.add_input_fn("input", || 2.0).unwrap();
        rustia.add_slot_read_fn("slot", || 3.0).unwrap();
        rustia
            .add_slot_write_fn("slot_write", |value: f64| value + 4.0)
            .unwrap();
        rustia
            .add_effect_fn("effect", |value: f64| value * 2.0)
            .unwrap();

        let result = rustia
            .run_owned("return pure() + input() + slot() + slot_write(1) + effect(3)")
            .unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 21.0);
    }

    #[test]
    fn deterministic_facade_rejects_external_or_nondeterministic_helpers() {
        let mut external = Rustia::new(Profile::DeterministicSim);
        let Err(Error::Vm(error)) = external.add_external_state_fn("clock", || 1.0) else {
            panic!("expected deterministic profile to reject external-state host function");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));

        let mut nondeterministic = Rustia::new(Profile::DeterministicSim);
        let Err(Error::Vm(error)) = nondeterministic.add_nondeterministic_fn("random", || 1.0)
        else {
            panic!("expected deterministic profile to reject nondeterministic host function");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));
    }

    #[test]
    fn standard_facade_accepts_external_and_nondeterministic_helpers() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        rustia.add_external_state_fn("clock", || 7.0).unwrap();
        rustia.add_nondeterministic_fn("random", || 11.0).unwrap();

        let result = rustia.run_owned("return clock() + random()").unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 18.0);
    }

    #[test]
    fn typed_host_function_arg_errors_include_function_and_position() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        rustia
            .add_fn("clamp", |x: f64, lo: f64, hi: f64| x.clamp(lo, hi))
            .unwrap();

        let wrong_type = rustia.run_owned("return clamp('bad', 0, 10)").unwrap_err();
        let wrong_type = wrong_type.to_string();
        assert!(
            wrong_type.contains("host function `clamp` argument 1: expected number, got string"),
            "{wrong_type}"
        );

        let missing_arg = rustia.run_owned("return clamp(4, 0)").unwrap_err();
        let missing_arg = missing_arg.to_string();
        assert!(
            missing_arg.contains("host function `clamp` argument 3: expected number, got nil"),
            "{missing_arg}"
        );
    }

    #[test]
    fn typed_host_functions_can_return_rustia_result_errors() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        rustia
            .add_fn("checked_div", |x: f64, y: f64| -> Result<f64> {
                if y == 0.0 {
                    return Err(Error::Embed("division by zero".to_owned()));
                }
                Ok(x / y)
            })
            .unwrap();

        let value: f64 = rustia.run("return checked_div(12, 3)").unwrap();
        assert_eq!(value, 4.0);

        let error = rustia
            .run_owned("return checked_div(12, 0)")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("host function `checked_div` return value: division by zero"),
            "{error}"
        );
    }

    #[test]
    fn exports_facade_retains_script_functions() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .compile_and_run_exports(
                "\
return {
    label = 'ai',
    decide = function(x)
        return x + 1
    end,
}
",
            )
            .unwrap();
        assert_eq!(exports.get::<String>(&mut rustia, "label").unwrap(), "ai");

        let decide = exports.function(&mut rustia, "decide").unwrap();
        let result: f64 = decide.call(&mut rustia, (41.0,)).unwrap();
        assert_eq!(result, 42.0);
    }

    #[test]
    fn retained_script_functions_return_vec_and_option_shapes() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .compile_and_run_exports(
                "\
return {
    values = function()
        return { 1, 2, 3 }
    end,
    maybe = function(enabled)
        if enabled then
            return 'scout'
        end
    end,
}
",
            )
            .unwrap();

        let values = exports.function(&mut rustia, "values").unwrap();
        let values: Vec<f64> = values.call(&mut rustia, ()).unwrap();
        assert_eq!(values, vec![1.0, 2.0, 3.0]);

        let maybe = exports.function(&mut rustia, "maybe").unwrap();
        let found: Option<String> = maybe.call(&mut rustia, (true,)).unwrap();
        let missing: Option<String> = maybe.call(&mut rustia, (false,)).unwrap();
        assert_eq!(found.as_deref(), Some("scout"));
        assert_eq!(missing, None);
    }

    #[test]
    fn retained_script_functions_support_eight_arguments_and_results() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .compile_and_run_exports(
                "\
return {
    rotate = function(a, b, c, d, e, f, g, h)
        return h, g, f, e, d, c, b, a
    end,
}
",
            )
            .unwrap();

        let rotate = exports.function(&mut rustia, "rotate").unwrap();
        let result: (f64, f64, f64, f64, f64, f64, f64, f64) = rotate
            .call(&mut rustia, (1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0))
            .unwrap();
        assert_eq!(result, (8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0));
    }

    #[test]
    fn builder_extension_installs_namespaced_deterministic_ops() {
        let extension = RustiaExtension::new("host")
            .fn_slot_read("bias", || 2.0)
            .fn_effect("emit", |value: f64| value + 1.0);
        let mut rustia = Rustia::builder()
            .profile(Profile::DeterministicSim)
            .extension(extension)
            .build()
            .unwrap();

        let result = rustia
            .run_owned("return host.bias() + host.emit(4)")
            .unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 7.0);
    }

    #[test]
    fn extension_reports_host_function_metadata() {
        let extension = RustiaExtension::new("host")
            .fn_pure("zero", || 1.0)
            .fn_pure("try_zero", || -> Result<f64> { Ok(1.0) })
            .fn_pure("split", |value: f64| (value, value > 0.0))
            .fn_with(
                "sum3",
                7,
                OpCapability::ReadsSlotState,
                |a: f64, b: f64, c: f64| a + b + c,
            );

        assert_eq!(extension.namespace(), Some("host"));
        assert_eq!(extension.fuel_policy(), RUSTIA_HOST_FUEL_POLICY);
        assert_eq!(
            extension.fuel_policy().abi_version,
            RUSTIA_HOST_FUEL_ABI_VERSION
        );
        assert_eq!(
            extension.fuel_policy().default_function_fuel,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL
        );
        assert_eq!(
            extension.fuel_policy().minimum_function_fuel,
            RUSTIA_MIN_HOST_FUNCTION_FUEL
        );
        let functions = extension.functions().collect::<Vec<_>>();
        assert_eq!(
            functions,
            vec![
                RustiaHostFunctionInfo {
                    name: "zero",
                    fuel_abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
                    fuel_cost: 1,
                    capability: OpCapability::Pure,
                    parameter_count: 0,
                    parameter_types: &[],
                    return_types: &["f64"],
                    fallible: false,
                },
                RustiaHostFunctionInfo {
                    name: "try_zero",
                    fuel_abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
                    fuel_cost: 1,
                    capability: OpCapability::Pure,
                    parameter_count: 0,
                    parameter_types: &[],
                    return_types: &["f64"],
                    fallible: true,
                },
                RustiaHostFunctionInfo {
                    name: "split",
                    fuel_abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
                    fuel_cost: 1,
                    capability: OpCapability::Pure,
                    parameter_count: 1,
                    parameter_types: &["f64"],
                    return_types: &["f64", "bool"],
                    fallible: false,
                },
                RustiaHostFunctionInfo {
                    name: "sum3",
                    fuel_abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
                    fuel_cost: 7,
                    capability: OpCapability::ReadsSlotState,
                    parameter_count: 3,
                    parameter_types: &["f64", "f64", "f64"],
                    return_types: &["f64"],
                    fallible: false,
                },
            ]
        );
    }

    #[test]
    fn extension_rejects_zero_fuel_host_functions() {
        let extension = RustiaExtension::new("host").fn_with("free", 0, OpCapability::Pure, || 1.0);
        let result = Rustia::builder().extension(extension).build();
        let Err(Error::Vm(error)) = result else {
            panic!("expected zero-fuel host function to be rejected");
        };
        let VmErrorKind::InvalidProgram { reason } = error.kind else {
            panic!("expected invalid-program error, got {error:?}");
        };
        assert_eq!(reason, "extension op fuel cost must be non-zero");
    }

    #[test]
    fn deterministic_builder_rejects_nondeterministic_extension() {
        let extension = RustiaExtension::new("host").fn_nondeterministic("clock", || 1.0);
        let result = Rustia::builder()
            .profile(Profile::DeterministicSim)
            .extension(extension)
            .build();
        let Err(Error::Vm(error)) = result else {
            panic!("expected deterministic profile to reject nondeterministic extension");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));
    }

    #[test]
    fn deterministic_builder_rejects_external_state_extension() {
        let extension = RustiaExtension::new("host").fn_external_state("clock", || 1.0);
        let result = Rustia::builder()
            .profile(Profile::DeterministicSim)
            .extension(extension)
            .build();
        let Err(Error::Vm(error)) = result else {
            panic!("expected deterministic profile to reject external-state extension");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));
    }

    #[test]
    fn hot_reload_compile_error_keeps_old_function_callable() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .hot_reload("ai.luau", "return { decide = function() return 1 end }")
            .unwrap();
        let decide = exports.function(&mut rustia, "decide").unwrap();
        let generation = rustia.generation();

        assert!(rustia.hot_reload("ai.luau", "return {").is_err());
        assert_eq!(rustia.generation(), generation);
        assert_eq!(decide.generation(), generation);
        let result: f64 = decide.call(&mut rustia, ()).unwrap();
        assert_eq!(result, 1.0);
    }

    #[test]
    fn retained_handles_are_versioned_across_successful_hot_reload() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let first_exports = rustia
            .hot_reload(
                "ai.luau",
                "return { label = 'old', decide = function() return 1 end }",
            )
            .unwrap();
        let first_generation = first_exports.generation();
        let first_decide = first_exports.function(&mut rustia, "decide").unwrap();

        let second_exports = rustia
            .hot_reload(
                "ai.luau",
                "return { label = 'new', decide = function() return 2 end }",
            )
            .unwrap();
        let second_generation = second_exports.generation();
        let second_decide = second_exports.function(&mut rustia, "decide").unwrap();

        assert!(second_generation > first_generation);
        assert_eq!(rustia.generation(), second_generation);
        assert_eq!(first_decide.generation(), first_generation);
        assert_eq!(second_decide.generation(), second_generation);
        assert!(!first_exports.is_current(&rustia));
        assert!(!first_decide.is_current(&rustia));
        assert!(second_exports.is_current(&rustia));
        assert!(second_decide.is_current(&rustia));
        let stale_exports = first_exports.ensure_current(&rustia).unwrap_err();
        assert!(
            stale_exports
                .to_string()
                .contains("stale exports handle from generation"),
            "{stale_exports}"
        );
        assert!(first_exports.value_current(&mut rustia).is_err());
        assert!(first_exports
            .get_current::<String>(&mut rustia, "label")
            .is_err());
        assert!(first_exports
            .function_current(&mut rustia, "decide")
            .is_err());
        assert_eq!(
            first_exports.get::<String>(&mut rustia, "label").unwrap(),
            "old"
        );
        assert_eq!(
            second_exports
                .get_current::<String>(&mut rustia, "label")
                .unwrap(),
            "new"
        );
        let current_decide = second_exports
            .function_current(&mut rustia, "decide")
            .unwrap();
        assert_eq!(current_decide.generation(), second_generation);

        let old_result: f64 = first_decide.call(&mut rustia, ()).unwrap();
        let new_result: f64 = second_decide.call(&mut rustia, ()).unwrap();
        assert_eq!(old_result, 1.0);
        assert_eq!(new_result, 2.0);

        let stale = first_decide
            .call_current::<_, f64>(&mut rustia, ())
            .unwrap_err();
        assert!(
            stale
                .to_string()
                .contains("stale function handle from generation"),
            "{stale}"
        );
        let current: f64 = second_decide.call_current(&mut rustia, ()).unwrap();
        assert_eq!(current, 2.0);
    }

    #[test]
    fn named_compile_preserves_source_name_in_runtime_diagnostics() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let error = rustia
            .hot_reload("bad_ai.luau", "local t = {}; t[0 / 0] = 1")
            .unwrap_err();
        assert!(error.to_string().contains("bad_ai.luau:1:"), "{error}");
    }

    #[test]
    fn named_run_helpers_preserve_source_name_in_runtime_diagnostics() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);

        let answer: f64 = rustia.run_named("inline.luau", "return 40 + 2").unwrap();
        assert_eq!(answer, 42.0);

        let owned = rustia.run_owned_named("owned.luau", "return 'ok'").unwrap();
        assert_eq!(String::from_rustia(owned).unwrap(), "ok");

        let exports = rustia
            .compile_and_run_exports_named("exports.luau", "return { answer = 42 }")
            .unwrap();
        assert_eq!(exports.get::<i32>(&mut rustia, "answer").unwrap(), 42);

        let generation = rustia.generation();
        let error = rustia
            .run_named::<f64>("bad_inline.luau", "local t = {}; t[0 / 0] = 1")
            .unwrap_err();
        assert!(error.to_string().contains("bad_inline.luau:1:"), "{error}");
        assert_eq!(rustia.generation(), generation);
    }

    #[test]
    fn public_errors_expose_source_span_and_runtime_location() {
        let Err(error) = Rustia::compile("compile-diag.luau", "break") else {
            panic!("break outside loop should fail compilation");
        };
        assert_eq!(error.source_span(), Some(SourceSpan { start: 0, end: 5 }));
        assert_eq!(error.source_location(), None);

        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let Err(error) = rustia.run_owned_named(
            "runtime-diag.luau",
            "\
local t = {}
t[0 / 0] = 1
",
        ) else {
            panic!("NaN table key should fail at runtime");
        };
        let location = error
            .source_location()
            .expect("runtime error should carry source location");
        assert_eq!(location.source, "runtime-diag.luau");
        assert_eq!(location.line, 2);
        assert_eq!(error.source_span(), None);
    }

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    struct ScenarioSpec {
        name: String,
        difficulty: u32,
        enemies: Vec<EnemySpec>,
    }

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    struct EnemySpec {
        kind: String,
        hp: u32,
    }

    #[test]
    fn serde_deserializes_exports_and_inserts_globals() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .compile_and_run_exports(
                "\
return {
    name = 'arena',
    difficulty = 3,
    enemies = {
        { kind = 'scout', hp = 5 },
        { kind = 'brute', hp = 9 },
    },
}
",
            )
            .unwrap();
        let spec: ScenarioSpec = exports.deserialize(&mut rustia).unwrap();
        assert_eq!(
            spec,
            ScenarioSpec {
                name: "arena".to_owned(),
                difficulty: 3,
                enemies: vec![
                    EnemySpec {
                        kind: "scout".to_owned(),
                        hp: 5
                    },
                    EnemySpec {
                        kind: "brute".to_owned(),
                        hp: 9
                    },
                ],
            }
        );
        let current_spec: ScenarioSpec = exports.deserialize_current(&mut rustia).unwrap();
        assert_eq!(current_spec, spec);

        rustia.insert_serde("config", &spec).unwrap();
        let value = rustia
            .run_owned("return config.enemies[2].kind .. ':' .. tostring(config.difficulty)")
            .unwrap();
        assert_eq!(String::from_rustia(value).unwrap(), "brute:3");
    }

    #[test]
    fn rustia_extension_macro_builds_capability_tagged_extension() {
        let extension = crate::rustia_extension!("host" {
            pure(fuel = 2) fn fixed_mul(a: i32, b: i32) -> i32 {
                a * b
            }

            effect fn emit(value: f64) -> f64 {
                value + 10.0
            }
        });
        let mut rustia = Rustia::builder()
            .profile(Profile::DeterministicSim)
            .extension(extension)
            .build()
            .unwrap();

        let value = rustia
            .run_owned("return host.fixed_mul(6, 7) + host.emit(1)")
            .unwrap();
        assert_eq!(f64::from_rustia(value).unwrap(), 53.0);
    }

    #[test]
    fn debug_traceback_for_errored_coroutine_reports_top_frame() {
        let source = "\
function halp(key, value)
    local t = {}
    t[key] = value
    return t
end

local co = coroutine.create(halp)
local ok, err = coroutine.resume(co, 0 / 0, 42)
return tostring(ok) .. \"|\" .. tostring(err) .. \"|\" .. debug.traceback(co)
";
        let chunk = parse_source(source).unwrap();
        let program = compile_with_options(
            &chunk,
            CompileOptions::default().with_source("debug.luau", source),
        )
        .unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        let traceback = vm.string_value(result).unwrap().unwrap();
        assert_eq!(
            traceback,
            "false|debug.luau:3: table index is NaN|debug.luau:3 function halp\n"
        );
    }

    #[test]
    fn debug_traceback_for_yielded_coroutine_reports_top_frame() {
        let source = "\
function gen()
    coroutine.yield(42)
end

local co = coroutine.create(gen)
local ok, value = coroutine.resume(co)
return tostring(ok) .. \"|\" .. tostring(value) .. \"|\" .. debug.traceback(co)
";
        let chunk = parse_source(source).unwrap();
        let program = compile_with_options(
            &chunk,
            CompileOptions::default().with_source("debug.luau", source),
        )
        .unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        let traceback = vm.string_value(result).unwrap().unwrap();
        assert_eq!(traceback, "true|42|debug.luau:2 function gen\n");
    }

    #[test]
    fn debug_info_line_defined_precedes_active_line() {
        let source = "\
function testlinedefined()
    local line = debug.info(1, \"l\")
    local linedefined = debug.info(testlinedefined, \"l\")
    return tostring(line) .. \"|\" .. tostring(linedefined)
end

return testlinedefined()
";
        let chunk = parse_source(source).unwrap();
        let program = compile_with_options(
            &chunk,
            CompileOptions::default().with_source("debug.luau", source),
        )
        .unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        let lines = vm.string_value(result).unwrap().unwrap();
        assert_eq!(lines, "2|1");
    }

    #[test]
    fn conformance_int64_userdata_supports_metamethods_and_value_field() {
        let settings = VmSettings {
            enable_conformance_int64_userdata: true,
            ..VmSettings::default()
        };
        let mut rustia = Rustia::with_settings(Profile::ReproducibleStandard, settings);
        let result = rustia
            .run_owned(
                "\
local v = int64(42)
v.value = 4
return tostring(v + 2) .. '|' .. tostring(v == int64(4)) .. '|' .. tostring(int64(-5) // 2)
",
            )
            .unwrap();
        assert_eq!(String::from_rustia(result).unwrap(), "6|true|-3");
    }

    #[test]
    fn conformance_native_userdata_supports_vec2_and_vertex_surface() {
        let settings = VmSettings {
            enable_conformance_native_userdata: true,
            ..VmSettings::default()
        };
        let mut rustia = Rustia::with_settings(Profile::ReproducibleStandard, settings);
        let result = rustia
            .run_owned(
                "\
local s = vec2(5, 4)
local t = vec2(1, 2)
local u = vec2(10, 20)
local x = -u + s * t
local m = u:Min(s)
local v = vertex(vector.create(1, 2, 3), vector.create(0.5, 0, 0.5), vec2(0.25, 0.5))
return x.X == -5 and x.Y == -12 and s:Dot(t) == 13 and m.X == 5 and m.Y == 4 and v.uv.X == 0.25 and v.normal.Z == 0.5
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn conformance_native_type_checks_validate_annotated_parameters() {
        let settings = VmSettings {
            enable_conformance_native_type_checks: true,
            ..VmSettings::default()
        };
        let mut rustia = Rustia::with_settings(Profile::ReproducibleStandard, settings);
        let result = rustia
            .run_owned(
                "\
local function add(a: number, b: number)
    assert(is_native())
    return a + b
end
local function checkbuffer(a: buffer?) return a == nil or type(a) == 'buffer' end
local ok = pcall(add, nil, 2)
return add(1, 3) == 4 and not ok and checkbuffer(nil) and checkbuffer(buffer.create(1))
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn coroutine_resume_arguments_return_from_yield() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .run_owned(
                "\
local co
local seen
local function body(a, ...)
    local arg = {...}
    for i = 1, #arg do
        seen = { coroutine.yield(table.unpack(arg[i])) }
    end
    return table.unpack(a)
end
co = coroutine.create(body)
local s, a, b, c, d = coroutine.resume(co, {1, 2, 3}, {}, {1}, {'a', 'b', 'c'})
local ok1 = s and a == nil and coroutine.status(co) == 'suspended'
s, a, b, c, d = coroutine.resume(co, 1, 2, 3)
local ok2 = s and #seen == 3 and seen[1] == 1 and seen[2] == 2 and seen[3] == 3
    and a == 1 and b == nil
s, a, b, c, d = coroutine.resume(co, 'done')
local ok3 = s and #seen == 1 and seen[1] == 'done'
    and a == 'a' and b == 'b' and c == 'c' and d == nil
s, a, b, c, d = coroutine.resume(co)
local ok4 = s and a == 1 and b == 2 and c == 3 and d == nil
    and coroutine.status(co) == 'dead'
return ok1 and ok2 and ok3 and ok4
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn coroutine_wrap_resumes_yield_call_results() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .run_owned(
                "\
local f = coroutine.wrap(function()
    local a, b = coroutine.yield('yielded')
    return a, b
end)
local first = f()
local second, third = f('resume', 42)
return first == 'yielded' and second == 'resume' and third == 42
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn coroutine_wrap_call_fast_path_observes_metatable_changes() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .run_owned(
                "\
local gen = coroutine.wrap(function()
    return 11
end)
local mt = getmetatable(gen)
local original = mt.__call
local replaced_self
mt.__call = function(self)
    replaced_self = self
    return 42
end
local replaced = gen()
mt.__call = original
local resumed = gen()
return replaced == 42 and replaced_self == gen and resumed == 11
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn pcall_can_yield_and_resume_successfully() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .run_owned(
                "\
local co = coroutine.create(function()
    return pcall(function()
        coroutine.yield('pause')
        return 1, 2, 3
    end)
end)
local ok, value = coroutine.resume(co)
local first = ok and value == 'pause' and coroutine.status(co) == 'suspended'
local done, protected, a, b, c = coroutine.resume(co)
return first and done and protected and a == 1 and b == 2 and c == 3
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn fixed_result_protected_calls_can_yield_and_resume_successfully() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .run_owned(
                "\
local co = coroutine.create(function()
    local p_ok, p_value = pcall(function()
        coroutine.yield('pcall')
        return 11, 12
    end)
    local x_ok, x_value = xpcall(function()
        coroutine.yield('xpcall')
        return 21, 22
    end, function()
        return -1
    end)
    return p_ok, p_value, x_ok, x_value
end)
local ok, value = coroutine.resume(co)
local first = ok and value == 'pcall'
ok, value = coroutine.resume(co)
local second = ok and value == 'xpcall'
local done, p_ok, p_value, x_ok, x_value = coroutine.resume(co)
return first and second and done and p_ok and p_value == 11 and x_ok and x_value == 21
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn nested_pcall_preserves_protected_results_after_yield() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .run_owned(
                "\
local co = coroutine.create(function()
    return pcall(function()
        coroutine.yield(1)
        return pcall(function()
            coroutine.yield(2)
            return 3
        end)
    end)
end)
local ok, a = coroutine.resume(co)
local first = ok and a == 1
ok, a = coroutine.resume(co)
local second = ok and a == 2
local done, outer, inner, value = coroutine.resume(co)
return first and second and done and outer and inner and value == 3
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn pcall_catches_error_after_yield() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .run_owned(
                "\
local co = coroutine.create(function()
    return pcall(function()
        coroutine.yield()
        error('foo')
    end)
end)
local ok = coroutine.resume(co)
local done, protected, err = coroutine.resume(co)
return ok and done and not protected and string.find(err, 'foo', 1, true) ~= nil
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }
}
