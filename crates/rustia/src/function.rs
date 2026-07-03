use crate::value::{
    embed_to_vm_error, load_owned_value, snapshot_value, FromRustia, FromRustiaMulti,
    IntoRustiaMulti, OwnedValue,
};
use crate::{Error, Result, Rustia};
use rustia_vm::{HostFunctionOp, OpCapability, Value as RawValue, ValueKind, Vm};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct Function {
    value: RawValue,
}

impl Function {
    pub(crate) const fn new(value: RawValue) -> Self {
        Self { value }
    }

    pub fn call<Args, Return>(&self, rustia: &mut Rustia, args: Args) -> Result<Return>
    where
        Args: IntoRustiaMulti,
        Return: FromRustiaMulti,
    {
        let raw_args = args
            .into_rustia_multi()
            .iter()
            .map(|value| load_owned_value(rustia.vm_mut(), value))
            .collect::<Result<Vec<_>>>()?;
        let raw_results = rustia
            .vm_mut()
            .call_retained_function(self.value, &raw_args)
            .map_err(Error::Vm)?;
        let results = raw_results
            .into_iter()
            .map(|value| snapshot_value(rustia.vm_mut(), value))
            .collect::<Result<Vec<_>>>()?;
        Return::from_rustia_multi(results)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Exports {
    value: RawValue,
}

impl Exports {
    pub(crate) const fn new(value: RawValue) -> Self {
        Self { value }
    }

    pub fn value(&self, rustia: &mut Rustia) -> Result<OwnedValue> {
        snapshot_value(rustia.vm_mut(), self.value)
    }

    pub fn deserialize<T>(&self, rustia: &mut Rustia) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let value = self.value(rustia)?;
        value.deserialize()
    }

    pub fn get<T>(&self, rustia: &mut Rustia, name: &str) -> Result<T>
    where
        T: FromRustia,
    {
        let value = self.raw_get(rustia, name)?;
        let value = snapshot_value(rustia.vm_mut(), value)?;
        T::from_rustia(value)
    }

    pub fn function(&self, rustia: &mut Rustia, name: &str) -> Result<Function> {
        let value = self.raw_get(rustia, name)?;
        if value.kind() != ValueKind::Function {
            return Err(Error::Embed(format!("export `{name}` is not a function")));
        }
        Ok(Function::new(value))
    }

    fn raw_get(&self, rustia: &mut Rustia, name: &str) -> Result<RawValue> {
        let key = rustia.vm_mut().string(name);
        rustia
            .vm_mut()
            .raw_table_get_value(self.value, key)
            .map_err(Error::Vm)
    }
}

pub trait IntoHostFunction<Args, Return>: Send + Sync + 'static {
    fn into_host_function(
        self,
        name: String,
        fuel_cost: u32,
        capability: OpCapability,
    ) -> HostFunctionOp;
}

impl<F, Return> IntoHostFunction<(), Return> for F
where
    F: Fn() -> Return + Send + Sync + 'static,
    Return: IntoRustiaMulti,
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
    }
}

macro_rules! impl_host_function {
    ($($name:ident),+ $(,)?) => {
        impl<Func, Return, $($name),+> IntoHostFunction<($($name,)+), Return> for Func
        where
            Func: Fn($($name),+) -> Return + Send + Sync + 'static,
            Return: IntoRustiaMulti,
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
            }
        }
    };
}

impl_host_function!(A);
impl_host_function!(A, B);
impl_host_function!(A, B, C);
impl_host_function!(A, B, C, D);

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
    Return: IntoRustiaMulti,
{
    value
        .into_rustia_multi()
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
