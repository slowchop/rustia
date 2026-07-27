use crate::value::{
    embed_to_vm_error, load_owned_value, snapshot_value, FromRustia, FromRustiaMulti,
    IntoRustiaMulti, OwnedValue,
};
use crate::{Error, Result, Rustia};
use rustia_vm::{HostFunctionOp, OpCapability, RetainedValue, Value as RawValue, ValueKind, Vm};
use std::any::type_name;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Function {
    value: RetainedValue,
    generation: u64,
    decision_epoch: u64,
}

impl Function {
    pub(crate) fn new(value: RetainedValue, generation: u64, decision_epoch: u64) -> Self {
        Self {
            value,
            generation,
            decision_epoch,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn decision_epoch(&self) -> u64 {
        self.decision_epoch
    }

    pub fn is_current(&self, rustia: &Rustia) -> bool {
        self.decision_epoch == rustia.decision_epoch() && self.generation == rustia.generation()
    }

    pub fn ensure_current(&self, rustia: &Rustia) -> Result<()> {
        ensure_current_epoch(
            "function handle",
            self.decision_epoch,
            rustia.decision_epoch(),
        )?;
        ensure_current_generation("function handle", self.generation, rustia.generation())
    }

    pub fn call<Args, Return>(&self, rustia: &mut Rustia, args: Args) -> Result<Return>
    where
        Args: IntoRustiaMulti,
        Return: FromRustiaMulti,
    {
        ensure_current_epoch(
            "function handle",
            self.decision_epoch,
            rustia.decision_epoch(),
        )?;
        ensure_same_vm("function handle", &self.value, rustia)?;
        let raw_args = args
            .into_rustia_multi()
            .iter()
            .map(|value| load_owned_value(rustia.vm_mut(), value))
            .collect::<Result<Vec<_>>>()?;
        let function = self.value.value(rustia.vm()).map_err(Error::Vm)?;
        let raw_results = rustia
            .vm_mut()
            .call_retained_function(function, &raw_args)
            .map_err(Error::Vm)?;
        let results = raw_results
            .into_iter()
            .map(|value| snapshot_value(rustia.vm_mut(), value))
            .collect::<Result<Vec<_>>>()?;
        Return::from_rustia_multi(results)
    }

    pub fn call_current<Args, Return>(&self, rustia: &mut Rustia, args: Args) -> Result<Return>
    where
        Args: IntoRustiaMulti,
        Return: FromRustiaMulti,
    {
        self.ensure_current(rustia)?;
        self.call(rustia, args)
    }
}

#[derive(Clone, Debug)]
pub struct Exports {
    value: RetainedValue,
    generation: u64,
    decision_epoch: u64,
}

impl Exports {
    pub(crate) fn new(value: RetainedValue, generation: u64, decision_epoch: u64) -> Self {
        Self {
            value,
            generation,
            decision_epoch,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn decision_epoch(&self) -> u64 {
        self.decision_epoch
    }

    pub fn is_current(&self, rustia: &Rustia) -> bool {
        self.decision_epoch == rustia.decision_epoch() && self.generation == rustia.generation()
    }

    pub fn ensure_current(&self, rustia: &Rustia) -> Result<()> {
        ensure_current_epoch(
            "exports handle",
            self.decision_epoch,
            rustia.decision_epoch(),
        )?;
        ensure_current_generation("exports handle", self.generation, rustia.generation())
    }

    pub fn value(&self, rustia: &mut Rustia) -> Result<OwnedValue> {
        ensure_current_epoch(
            "exports handle",
            self.decision_epoch,
            rustia.decision_epoch(),
        )?;
        ensure_same_vm("exports handle", &self.value, rustia)?;
        let value = self.value.value(rustia.vm()).map_err(Error::Vm)?;
        snapshot_value(rustia.vm_mut(), value)
    }

    pub fn value_current(&self, rustia: &mut Rustia) -> Result<OwnedValue> {
        self.ensure_current(rustia)?;
        self.value(rustia)
    }

    pub fn deserialize<T>(&self, rustia: &mut Rustia) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let value = self.value(rustia)?;
        value.deserialize()
    }

    pub fn deserialize_current<T>(&self, rustia: &mut Rustia) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        self.ensure_current(rustia)?;
        self.deserialize(rustia)
    }

    pub fn get<T>(&self, rustia: &mut Rustia, name: &str) -> Result<T>
    where
        T: FromRustia,
    {
        let value = self.raw_get(rustia, name)?;
        let value = snapshot_value(rustia.vm_mut(), value)?;
        T::from_rustia(value)
    }

    pub fn get_current<T>(&self, rustia: &mut Rustia, name: &str) -> Result<T>
    where
        T: FromRustia,
    {
        self.ensure_current(rustia)?;
        self.get(rustia, name)
    }

    pub fn function(&self, rustia: &mut Rustia, name: &str) -> Result<Function> {
        let value = self.raw_get(rustia, name)?;
        if value.kind() != ValueKind::Function {
            return Err(Error::Embed(format!("export `{name}` is not a function")));
        }
        let value = rustia.vm_mut().retain_value(value);
        Ok(Function::new(value, self.generation, self.decision_epoch))
    }

    pub fn function_current(&self, rustia: &mut Rustia, name: &str) -> Result<Function> {
        self.ensure_current(rustia)?;
        self.function(rustia, name)
    }

    fn raw_get(&self, rustia: &mut Rustia, name: &str) -> Result<RawValue> {
        ensure_current_epoch(
            "exports handle",
            self.decision_epoch,
            rustia.decision_epoch(),
        )?;
        ensure_same_vm("exports handle", &self.value, rustia)?;
        let key = rustia.vm_mut().string(name);
        let table = self.value.value(rustia.vm()).map_err(Error::Vm)?;
        rustia
            .vm_mut()
            .raw_table_get_value(table, key)
            .map_err(Error::Vm)
    }
}

fn ensure_current_epoch(label: &str, handle_epoch: u64, current_epoch: u64) -> Result<()> {
    if handle_epoch == current_epoch {
        return Ok(());
    }
    Err(Error::Embed(format!(
        "stale {label} from decision epoch {handle_epoch}; current decision epoch is {current_epoch}"
    )))
}

fn ensure_same_vm(label: &str, value: &RetainedValue, rustia: &Rustia) -> Result<()> {
    if value.belongs_to(rustia.vm()) {
        return Ok(());
    }
    Err(Error::Embed(format!(
        "{label} belongs to a different Rustia VM"
    )))
}

fn ensure_current_generation(
    label: &str,
    handle_generation: u64,
    current_generation: u64,
) -> Result<()> {
    if handle_generation == current_generation {
        return Ok(());
    }
    Err(Error::Embed(format!(
        "stale {label} from generation {handle_generation}; current generation is {current_generation}"
    )))
}

pub trait IntoHostFunction<Args, Return>: Send + Sync + 'static {
    fn into_host_function(
        self,
        name: String,
        fuel_cost: u32,
        capability: OpCapability,
    ) -> HostFunctionOp;
}

pub trait IntoHostReturn {
    fn into_host_return(self) -> Result<Vec<OwnedValue>>;

    fn rustia_type_names() -> Vec<&'static str>
    where
        Self: Sized;

    fn is_fallible() -> bool
    where
        Self: Sized,
    {
        false
    }
}

impl<T> IntoHostReturn for T
where
    T: IntoRustiaMulti,
{
    fn into_host_return(self) -> Result<Vec<OwnedValue>> {
        Ok(self.into_rustia_multi())
    }

    fn rustia_type_names() -> Vec<&'static str> {
        T::rustia_type_names()
    }
}

impl<T> IntoHostReturn for Result<T>
where
    T: IntoRustiaMulti,
{
    fn into_host_return(self) -> Result<Vec<OwnedValue>> {
        self.map(IntoRustiaMulti::into_rustia_multi)
    }

    fn rustia_type_names() -> Vec<&'static str> {
        T::rustia_type_names()
    }

    fn is_fallible() -> bool {
        true
    }
}

impl<F, Return> IntoHostFunction<(), Return> for F
where
    F: Fn() -> Return + Send + Sync + 'static,
    Return: IntoHostReturn,
{
    fn into_host_function(
        self,
        name: String,
        fuel_cost: u32,
        capability: OpCapability,
    ) -> HostFunctionOp {
        let diagnostic_name = name.clone();
        HostFunctionOp::new(
            name,
            fuel_cost,
            capability,
            Arc::new(move |vm, _args| host_return_to_raw(vm, &diagnostic_name, self())),
        )
        .with_signature(
            0,
            Vec::new(),
            Return::rustia_type_names(),
            Return::is_fallible(),
        )
    }
}

macro_rules! impl_host_function {
    ($parameter_count:expr; $($name:ident),+ $(,)?) => {
        impl<Func, Return, $($name),+> IntoHostFunction<($($name,)+), Return> for Func
        where
            Func: Fn($($name),+) -> Return + Send + Sync + 'static,
            Return: IntoHostReturn,
            $($name: FromRustia,)+
        {
            #[allow(non_snake_case)]
            fn into_host_function(
                self,
                name: String,
                fuel_cost: u32,
                capability: OpCapability,
            ) -> HostFunctionOp {
                let diagnostic_name = name.clone();
                HostFunctionOp::new(
                    name,
                    fuel_cost,
                    capability,
                    Arc::new(move |vm, args| {
                        let mut index = 0usize;
                        $(
                            let $name = host_arg_from_raw(vm, args, index, &diagnostic_name)?;
                            index += 1;
                        )+
                        let _ = index;
                        host_return_to_raw(vm, &diagnostic_name, self($($name),+))
                    }),
                )
                .with_signature(
                    $parameter_count,
                    vec![$(type_name::<$name>()),+],
                    Return::rustia_type_names(),
                    Return::is_fallible(),
                )
            }
        }
    };
}

impl_host_function!(1; A);
impl_host_function!(2; A, B);
impl_host_function!(3; A, B, C);
impl_host_function!(4; A, B, C, D);
impl_host_function!(5; A, B, C, D, E);
impl_host_function!(6; A, B, C, D, E, F);
impl_host_function!(7; A, B, C, D, E, F, G);
impl_host_function!(8; A, B, C, D, E, F, G, H);

fn host_arg_from_raw<T>(
    vm: &mut Vm,
    args: &[RawValue],
    index: usize,
    function_name: &str,
) -> rustia_vm::Result<T>
where
    T: FromRustia,
{
    let value = args.get(index).copied().unwrap_or_else(RawValue::nil);
    let owned = snapshot_value(vm, value).map_err(|error| {
        host_embed_error(function_name, format!("argument {}", index + 1), error)
    })?;
    T::from_rustia(owned)
        .map_err(|error| host_embed_error(function_name, format!("argument {}", index + 1), error))
}

fn host_return_to_raw<Return>(
    vm: &mut Vm,
    function_name: &str,
    value: Return,
) -> rustia_vm::Result<Vec<RawValue>>
where
    Return: IntoHostReturn,
{
    value
        .into_host_return()
        .map_err(|error| host_embed_error(function_name, "return value", error))?
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            load_owned_value(vm, &value).map_err(|error| {
                host_embed_error(function_name, format!("return value {}", index + 1), error)
            })
        })
        .collect()
}

fn host_embed_error(function_name: &str, slot: impl AsRef<str>, error: Error) -> rustia_vm::Error {
    embed_to_vm_error(Error::Embed(format!(
        "host function `{function_name}` {}: {error}",
        slot.as_ref()
    )))
}
