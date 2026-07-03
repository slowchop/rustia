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

pub use extension_api::{RustiaBuilder, RustiaExtension, RustiaHostFunctionInfo};
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
        Error, FromRustia, OpCapability, OwnedKey, OwnedTable, OwnedValue, Profile, Rustia,
        RustiaExtension, RustiaHostFunctionInfo, VmErrorKind, VmSettings,
    };
    use rustia_compiler::{compile_with_options, CompileOptions};
    use rustia_parser::parse_source;
    use rustia_vm::Vm;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

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
            .insert_global(
                "named",
                OwnedTable::entries([(
                    OwnedKey::String("kind".to_owned()),
                    OwnedValue::String("scout".to_owned()),
                )]),
            )
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
                OwnedTable::entries([(
                    OwnedKey::String("kind".to_owned()),
                    OwnedValue::String("scout".to_owned()),
                )])
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
            .fn_pure("split", |value: f64| (value, value > 0.0))
            .fn_with(
                "sum3",
                7,
                OpCapability::ReadsSlotState,
                |a: f64, b: f64, c: f64| a + b + c,
            );

        assert_eq!(extension.namespace(), Some("host"));
        let functions = extension.functions().collect::<Vec<_>>();
        assert_eq!(
            functions,
            vec![
                RustiaHostFunctionInfo {
                    name: "zero",
                    fuel_cost: 1,
                    capability: OpCapability::Pure,
                    parameter_count: 0,
                    parameter_types: &[],
                    return_types: &["f64"],
                },
                RustiaHostFunctionInfo {
                    name: "split",
                    fuel_cost: 1,
                    capability: OpCapability::Pure,
                    parameter_count: 1,
                    parameter_types: &["f64"],
                    return_types: &["f64", "bool"],
                },
                RustiaHostFunctionInfo {
                    name: "sum3",
                    fuel_cost: 7,
                    capability: OpCapability::ReadsSlotState,
                    parameter_count: 3,
                    parameter_types: &["f64", "f64", "f64"],
                    return_types: &["f64"],
                },
            ]
        );
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
            .hot_reload("ai.luau", "return { decide = function() return 1 end }")
            .unwrap();
        let first_generation = first_exports.generation();
        let first_decide = first_exports.function(&mut rustia, "decide").unwrap();

        let second_exports = rustia
            .hot_reload("ai.luau", "return { decide = function() return 2 end }")
            .unwrap();
        let second_generation = second_exports.generation();
        let second_decide = second_exports.function(&mut rustia, "decide").unwrap();

        assert!(second_generation > first_generation);
        assert_eq!(rustia.generation(), second_generation);
        assert_eq!(first_decide.generation(), first_generation);
        assert_eq!(second_decide.generation(), second_generation);

        let old_result: f64 = first_decide.call(&mut rustia, ()).unwrap();
        let new_result: f64 = second_decide.call(&mut rustia, ()).unwrap();
        assert_eq!(old_result, 1.0);
        assert_eq!(new_result, 2.0);
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
