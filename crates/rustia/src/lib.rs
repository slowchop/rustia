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

pub use extension_api::{RustiaBuilder, RustiaExtension};
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
}

impl Rustia {
    pub fn builder() -> RustiaBuilder {
        RustiaBuilder::new()
    }

    pub fn new(profile: Profile) -> Self {
        Self {
            vm: Vm::new(profile),
        }
    }

    pub fn with_settings(profile: Profile, settings: VmSettings) -> Self {
        Self {
            vm: Vm::with_settings(profile, settings),
        }
    }

    pub fn compile_and_run(&mut self, source: &str) -> Result<Exports> {
        self.compile_and_run_raw(source).map(Exports::new)
    }

    pub fn compile_and_run_exports(&mut self, source: &str) -> Result<Exports> {
        self.compile_and_run(source)
    }

    pub fn run_owned(&mut self, source: &str) -> Result<OwnedValue> {
        let value = self.compile_and_run_raw(source)?;
        value::snapshot_value(&mut self.vm, value)
    }

    pub(crate) fn compile_and_run_raw(&mut self, source: &str) -> Result<rustia_vm::Value> {
        let chunk = rustia_parser::parse_source(source).map_err(Error::Source)?;
        let program = rustia_compiler::compile(&chunk).map_err(Error::Compile)?;
        self.vm.run(&program).map_err(Error::Vm)
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
        self.vm
            .run(&module.program)
            .map(Exports::new)
            .map_err(Error::Vm)
    }

    pub fn hot_reload(&mut self, source_name: impl Into<String>, source: &str) -> Result<Exports> {
        let module = Self::compile(source_name, source)?;
        self.install(module)
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

#[derive(Debug)]
pub enum Error {
    Source(SourceError),
    Compile(CompileError),
    Vm(VmError),
    Embed(String),
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
        Error, FromRustia, OwnedValue, Profile, Rustia, RustiaExtension, VmErrorKind, VmSettings,
    };
    use rustia_compiler::{compile_with_options, CompileOptions};
    use rustia_parser::parse_source;
    use rustia_vm::Vm;
    use serde::{Deserialize, Serialize};

    #[test]
    fn compile_and_run_smoke() {
        let mut rustia = Rustia::new(Profile::DeterministicSim);
        let result = rustia
            .run_owned("local x: number = 6 * 7; return x")
            .unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 42.0);
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
    fn hot_reload_compile_error_keeps_old_function_callable() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .hot_reload("ai.luau", "return { decide = function() return 1 end }")
            .unwrap();
        let decide = exports.function(&mut rustia, "decide").unwrap();

        assert!(rustia.hot_reload("ai.luau", "return {").is_err());
        let result: f64 = decide.call(&mut rustia, ()).unwrap();
        assert_eq!(result, 1.0);
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
