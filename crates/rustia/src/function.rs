use crate::live_value::{ensure_current_epoch, Value as LiveValue};
use crate::serde_value::from_live;
use crate::value::{
    embed_to_vm_error, load_owned_value, snapshot_value, FromRustia, FromRustiaMulti, IntoRustia,
    IntoRustiaMulti, OwnedValue,
};
use crate::{Error, HostSchema, Invocation, Result, Rustia};
use rustia_vm::{HostFunctionOp, OpCapability, RetainedValue, Value as RawValue, ValueKind, Vm};
use std::any::type_name;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Function {
    value: LiveValue,
    decision_epoch: u64,
}

impl Function {
    pub(crate) fn new(value: RetainedValue, decision_epoch: u64) -> Self {
        Self {
            value: LiveValue::new(value, decision_epoch),
            decision_epoch,
        }
    }

    pub(crate) fn from_value(value: LiveValue) -> Self {
        Self {
            decision_epoch: value.decision_epoch(),
            value,
        }
    }

    pub fn as_value(&self) -> LiveValue {
        self.value.clone()
    }

    pub fn decision_epoch(&self) -> u64 {
        self.decision_epoch
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
        let raw_args = args
            .into_rustia_multi()?
            .iter()
            .map(|value| load_owned_value(rustia.vm_mut(), value))
            .collect::<Result<Vec<_>>>()?;
        let function = self.value.raw(rustia.vm(), "function handle")?;
        let raw_results = rustia
            .vm_mut()
            .call_retained_function(function, &raw_args)
            .map_err(Error::from)?;
        let results = raw_results
            .into_iter()
            .map(|value| snapshot_value(rustia.vm_mut(), value))
            .collect::<Result<Vec<_>>>()?;
        Return::from_rustia_multi(results)
    }

    /// Calls this function with rooted live values and retains every result.
    pub fn call_values(&self, rustia: &mut Rustia, args: &[LiveValue]) -> Result<Vec<LiveValue>> {
        ensure_current_epoch(
            "function handle",
            self.decision_epoch,
            rustia.decision_epoch(),
        )?;
        let function = self.value.raw(rustia.vm(), "function handle")?;
        let raw_args = args
            .iter()
            .map(|value| value.raw(rustia.vm(), "function argument handle"))
            .collect::<Result<Vec<_>>>()?;
        let results = rustia
            .vm_mut()
            .call_retained_function(function, &raw_args)
            .map_err(Error::from)?;
        LiveValue::retain_many(rustia.vm_mut(), &results)
    }

    /// Calls this guest function inside a typed host invocation. Argument
    /// loading, guest execution, result snapshotting, and typed conversion are
    /// one effect-transaction scope.
    pub fn call_in<C, Args, Return>(
        &self,
        invocation: &mut Invocation<'_, C>,
        args: Args,
    ) -> Result<Return>
    where
        C: HostSchema,
        Args: IntoRustiaMulti,
        Return: FromRustiaMulti,
    {
        ensure_current_epoch(
            "function handle",
            self.decision_epoch,
            invocation.vm.decision_epoch(),
        )?;
        let function = self.value.raw(invocation.vm, "function handle")?;
        invocation.convert_call(function, args)
    }

    /// Calls this function with live values inside a typed host invocation.
    pub fn call_values_in<C>(
        &self,
        invocation: &mut Invocation<'_, C>,
        args: &[LiveValue],
    ) -> Result<Vec<LiveValue>>
    where
        C: HostSchema,
    {
        ensure_current_epoch(
            "function handle",
            self.decision_epoch,
            invocation.vm.decision_epoch(),
        )?;
        invocation.scoped(|invocation| {
            let function = self.value.raw(invocation.vm, "function handle")?;
            let raw_args = args
                .iter()
                .map(|value| value.raw(invocation.vm, "function argument handle"))
                .collect::<Result<Vec<_>>>()?;
            let results = invocation.call_retained_raw(function, &raw_args)?;
            LiveValue::retain_many(invocation.vm, &results)
        })
    }
}

#[derive(Clone, Debug)]
pub struct Exports {
    value: LiveValue,
    decision_epoch: u64,
}

impl Exports {
    pub(crate) fn new(value: RetainedValue, decision_epoch: u64) -> Self {
        Self {
            value: LiveValue::new(value, decision_epoch),
            decision_epoch,
        }
    }

    pub fn as_value(&self) -> LiveValue {
        self.value.clone()
    }

    pub fn as_table<C>(&self, rustia: &Rustia<C>) -> Result<Option<crate::Table>>
    where
        C: HostSchema,
    {
        self.value.as_table(rustia)
    }

    pub fn decision_epoch(&self) -> u64 {
        self.decision_epoch
    }

    pub fn value(&self, rustia: &mut Rustia) -> Result<OwnedValue> {
        ensure_current_epoch(
            "exports handle",
            self.decision_epoch,
            rustia.decision_epoch(),
        )?;
        let value = self.value.raw(rustia.vm(), "exports handle")?;
        snapshot_value(rustia.vm_mut(), value)
    }

    pub fn value_in<C>(&self, invocation: &mut Invocation<'_, C>) -> Result<OwnedValue>
    where
        C: HostSchema,
    {
        ensure_current_epoch(
            "exports handle",
            self.decision_epoch,
            invocation.vm.decision_epoch(),
        )?;
        invocation.scoped(|invocation| {
            let value = self.value.raw(invocation.vm, "exports handle")?;
            snapshot_value(invocation.vm, value)
        })
    }

    /// Deserializes the returned value directly without building an owned graph snapshot.
    pub fn deserialize<T>(&self, rustia: &mut Rustia<impl HostSchema>) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        ensure_current_epoch(
            "exports handle",
            self.decision_epoch,
            rustia.decision_epoch(),
        )?;
        let value = self.value.raw(rustia.vm(), "exports handle")?;
        from_live(rustia.vm_mut(), value)
    }

    pub fn get<T>(&self, rustia: &mut Rustia, name: &str) -> Result<T>
    where
        T: FromRustia,
    {
        let value = self.raw_get(rustia, name)?;
        let value = snapshot_value(rustia.vm_mut(), value)?;
        T::from_rustia(value)
    }

    /// Reads a string-keyed export without snapshotting it.
    pub fn get_live(&self, rustia: &mut Rustia, name: &str) -> Result<LiveValue> {
        let value = self.raw_get(rustia, name)?;
        LiveValue::retain(rustia.vm_mut(), value)
    }

    pub fn get_in<C, T>(&self, invocation: &mut Invocation<'_, C>, name: &str) -> Result<T>
    where
        C: HostSchema,
        T: FromRustia,
    {
        invocation.scoped(|invocation| {
            let value = self.raw_get_in(invocation, name)?;
            let value = snapshot_value(invocation.vm, value)?;
            T::from_rustia(value)
        })
    }

    pub fn get_live_in<C>(
        &self,
        invocation: &mut Invocation<'_, C>,
        name: &str,
    ) -> Result<LiveValue>
    where
        C: HostSchema,
    {
        invocation.scoped(|invocation| {
            let value = self.raw_get_in(invocation, name)?;
            LiveValue::retain(invocation.vm, value)
        })
    }

    pub fn function(&self, rustia: &mut Rustia, name: &str) -> Result<Function> {
        let value = self.raw_get(rustia, name)?;
        if value.kind() != ValueKind::Function {
            return Err(Error::embed(format!("export `{name}` is not a function")));
        }
        let value = rustia.vm_mut().retain_value(value).map_err(Error::from)?;
        Ok(Function::new(value, self.decision_epoch))
    }

    pub fn function_in<C>(&self, invocation: &mut Invocation<'_, C>, name: &str) -> Result<Function>
    where
        C: HostSchema,
    {
        invocation.scoped(|invocation| {
            let value = self.raw_get_in(invocation, name)?;
            if value.kind() != ValueKind::Function {
                return Err(Error::embed(format!("export `{name}` is not a function")));
            }
            let value = invocation.vm.retain_value(value).map_err(Error::from)?;
            Ok(Function::new(value, self.decision_epoch))
        })
    }

    fn raw_get(&self, rustia: &mut Rustia, name: &str) -> Result<RawValue> {
        ensure_current_epoch(
            "exports handle",
            self.decision_epoch,
            rustia.decision_epoch(),
        )?;
        let key = rustia.vm_mut().string(name);
        let table = self.value.raw(rustia.vm(), "exports handle")?;
        rustia
            .vm_mut()
            .raw_table_get_value(table, key)
            .map_err(Error::from)
    }

    fn raw_get_in<C>(&self, invocation: &mut Invocation<'_, C>, name: &str) -> Result<RawValue>
    where
        C: HostSchema,
    {
        ensure_current_epoch(
            "exports handle",
            self.decision_epoch,
            invocation.vm.decision_epoch(),
        )?;
        let key = invocation.vm.string(name);
        let table = self.value.raw(invocation.vm, "exports handle")?;
        invocation
            .vm
            .raw_table_get_value(table, key)
            .map_err(Error::from)
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

pub enum HostReturnValue {
    Owned(OwnedValue),
    Live(LiveValue),
}

pub trait IntoHostValue {
    fn into_host_value(self) -> Result<HostReturnValue>;
}

impl<T: IntoRustia> IntoHostValue for T {
    fn into_host_value(self) -> Result<HostReturnValue> {
        self.into_rustia().map(HostReturnValue::Owned)
    }
}

impl IntoHostValue for LiveValue {
    fn into_host_value(self) -> Result<HostReturnValue> {
        Ok(HostReturnValue::Live(self))
    }
}

impl IntoHostValue for crate::Table {
    fn into_host_value(self) -> Result<HostReturnValue> {
        self.as_value().into_host_value()
    }
}

impl IntoHostValue for Function {
    fn into_host_value(self) -> Result<HostReturnValue> {
        self.as_value().into_host_value()
    }
}

/// Callback arguments may use either ordinary owned conversions or rooted live handles.
pub trait FromHostArgument: Sized {
    fn from_host_argument(vm: &mut Vm, value: RawValue) -> Result<Self>;
}

impl<T: FromRustia> FromHostArgument for T {
    fn from_host_argument(vm: &mut Vm, value: RawValue) -> Result<Self> {
        T::from_rustia(snapshot_value(vm, value)?)
    }
}

impl FromHostArgument for LiveValue {
    fn from_host_argument(vm: &mut Vm, value: RawValue) -> Result<Self> {
        LiveValue::retain(vm, value)
    }
}

impl FromHostArgument for crate::Table {
    fn from_host_argument(vm: &mut Vm, value: RawValue) -> Result<Self> {
        if value.kind() != ValueKind::Table {
            return Err(Error::embed("expected table".to_owned()));
        }
        Ok(Self::new(LiveValue::retain(vm, value)?))
    }
}

impl FromHostArgument for Function {
    fn from_host_argument(vm: &mut Vm, value: RawValue) -> Result<Self> {
        if value.kind() != ValueKind::Function {
            return Err(Error::embed("expected function".to_owned()));
        }
        Ok(Self::from_value(LiveValue::retain(vm, value)?))
    }
}

pub trait IntoHostReturn {
    fn into_host_return(self) -> Result<Vec<HostReturnValue>>;

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

macro_rules! impl_single_host_return {
    ($($ty:ty),+ $(,)?) => {$(
        impl IntoHostReturn for $ty {
            fn into_host_return(self) -> Result<Vec<HostReturnValue>> {
                Ok(vec![self.into_host_value()?])
            }
            fn rustia_type_names() -> Vec<&'static str> {
                vec![type_name::<Self>()]
            }
        }
    )+};
}

impl_single_host_return!(
    OwnedValue,
    crate::OwnedTable,
    bool,
    f64,
    f32,
    i8,
    i16,
    i32,
    i64,
    isize,
    u8,
    u16,
    u32,
    u64,
    usize,
    String,
    &str,
    LiveValue,
    crate::Table,
    Function
);

macro_rules! impl_owned_container_host_return {
    ($ty:ty) => {
        impl<T: IntoRustia> IntoHostReturn for $ty {
            fn into_host_return(self) -> Result<Vec<HostReturnValue>> {
                Ok(vec![self.into_host_value()?])
            }
            fn rustia_type_names() -> Vec<&'static str> {
                vec![type_name::<Self>()]
            }
        }
    };
}

impl_owned_container_host_return!(Option<T>);
impl_owned_container_host_return!(Vec<T>);
impl_owned_container_host_return!(std::collections::BTreeMap<String, T>);

impl IntoHostReturn for () {
    fn into_host_return(self) -> Result<Vec<HostReturnValue>> {
        Ok(Vec::new())
    }
    fn rustia_type_names() -> Vec<&'static str> {
        Vec::new()
    }
}

macro_rules! impl_tuple_host_return {
    ($($name:ident),+ $(,)?) => {
        impl<$($name: IntoHostValue),+> IntoHostReturn for ($($name,)+) {
            #[allow(non_snake_case)]
            fn into_host_return(self) -> Result<Vec<HostReturnValue>> {
                let ($($name,)+) = self;
                Ok(vec![$($name.into_host_value()?,)+])
            }
            fn rustia_type_names() -> Vec<&'static str> {
                vec![$(type_name::<$name>()),+]
            }
        }
    };
}

impl_tuple_host_return!(A);
impl_tuple_host_return!(A, B);
impl_tuple_host_return!(A, B, C);
impl_tuple_host_return!(A, B, C, D);
impl_tuple_host_return!(A, B, C, D, E);
impl_tuple_host_return!(A, B, C, D, E, F);
impl_tuple_host_return!(A, B, C, D, E, F, G);
impl_tuple_host_return!(A, B, C, D, E, F, G, H);

impl<T: IntoHostReturn> IntoHostReturn for Result<T> {
    fn into_host_return(self) -> Result<Vec<HostReturnValue>> {
        self.and_then(IntoHostReturn::into_host_return)
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
            $($name: FromHostArgument,)+
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

pub(crate) fn host_arg_from_raw<T>(
    vm: &mut Vm,
    args: &[RawValue],
    index: usize,
    function_name: &str,
) -> rustia_vm::Result<T>
where
    T: FromHostArgument,
{
    let value = args.get(index).copied().unwrap_or_else(RawValue::nil);
    T::from_host_argument(vm, value)
        .map_err(|error| host_embed_error(function_name, format!("argument {}", index + 1), error))
}

pub(crate) fn host_return_to_raw<Return>(
    vm: &mut Vm,
    function_name: &str,
    value: Return,
) -> rustia_vm::Result<Vec<RawValue>>
where
    Return: IntoHostReturn,
{
    let values = value
        .into_host_return()
        .map_err(|error| host_embed_error(function_name, "return value", error))?;
    // Validate every live handle before allocating an owned result. Keep all live
    // values and each newly loaded owned value rooted through later conversions.
    for value in &values {
        if let HostReturnValue::Live(value) = value {
            value
                .raw(vm, "host function result")
                .map_err(embed_to_vm_error)?;
        }
    }
    let mut roots = Vec::new();
    let mut results = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let raw = match value {
            HostReturnValue::Owned(value) => {
                let raw = load_owned_value(vm, value).map_err(|error| {
                    host_embed_error(function_name, format!("return value {}", index + 1), error)
                })?;
                if index + 1 < values.len()
                    && !matches!(
                        raw.kind(),
                        ValueKind::Nil | ValueKind::Boolean | ValueKind::Number
                    )
                {
                    roots.push(vm.retain_value(raw)?);
                }
                raw
            }
            HostReturnValue::Live(value) => value
                .raw(vm, "host function result")
                .map_err(embed_to_vm_error)?,
        };
        results.push(raw);
    }
    Ok(results)
}

fn host_embed_error(function_name: &str, slot: impl AsRef<str>, error: Error) -> rustia_vm::Error {
    match error {
        Error {
            kind: crate::ErrorKind::Vm(error),
            ..
        } => *error,
        error => embed_to_vm_error(
            error.with_context(format!("host function `{function_name}` {}", slot.as_ref())),
        ),
    }
}
