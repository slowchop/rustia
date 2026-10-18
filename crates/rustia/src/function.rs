use crate::live_value::Value as LiveValue;
use crate::serde_value::from_live;
use crate::value::{
    embed_to_vm_error, load_owned_value, snapshot_value, FromRustia, FromRustiaMulti, IntoRustia,
    IntoRustiaMulti, OwnedValue,
};
use crate::{Error, HostSchema, Invocation, Result, Rustia};
use rustia_vm::{HostFunctionOp, OpCapability, RetainedValue, Value as RawValue, ValueKind, Vm};
use std::any::type_name;
use std::sync::Arc;

/// A live script or Rust function kept alive by a Rust handle.
///
/// Call it with ordinary Rust arguments using [`call`](Self::call), or preserve
/// script object identity with [`call_values`](Self::call_values). Clones refer
/// to the same function. A handle belongs to one VM, survives module replacement,
/// and becomes invalid after [`Rustia::reset_decision`]. Using it with another
/// VM or after reset returns an error.
///
/// ```
/// use rustia::Rustia;
///
/// let mut lua = Rustia::builder().build()?;
/// let exports = lua.load("return { double = function(n) return n * 2 end }").install()?;
/// let double = exports.function(&mut lua, "double")?;
/// assert_eq!(double.call::<_, i64>(&mut lua, 21)?, 42);
/// # Ok::<(), rustia::Error>(())
/// ```
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

    /// Clones the underlying live value handle.
    pub fn as_value(&self) -> LiveValue {
        self.value.clone()
    }

    /// Returns the decision epoch in which this handle was retained.
    pub fn decision_epoch(&self) -> u64 {
        self.decision_epoch
    }

    /// Calls the function with Rust arguments and copies its results into Rust.
    ///
    /// Pass `()` for no arguments or a tuple for multiple arguments. The return
    /// type selects the results through [`FromRustiaMulti`]. Use
    /// [`call_values`](Self::call_values) for results that cannot be copied into
    /// [`OwnedValue`], such as functions or cyclic tables. Every result is copied
    /// before selection, including extra results and results discarded by `()`.
    pub fn call<Args, Return>(&self, rustia: &mut Rustia, args: Args) -> Result<Return>
    where
        Args: IntoRustiaMulti,
        Return: FromRustiaMulti,
    {
        let function = self.value.raw(rustia.vm(), "function handle")?;
        let raw_args = args
            .into_rustia_multi()?
            .iter()
            .map(|value| load_owned_value(rustia.vm_mut(), value))
            .collect::<Result<Vec<_>>>()?;
        let raw_results = rustia
            .vm_mut()
            .call_retained_function(function, &raw_args)
            .map_err(Error::from_engine)?;
        let results = raw_results
            .into_iter()
            .map(|value| snapshot_value(rustia.vm_mut(), value))
            .collect::<Result<Vec<_>>>()?;
        Return::from_rustia_multi(results)
    }

    /// Calls the function with live arguments and returns live handles for every result.
    ///
    /// All handles must belong to this VM and its current decision epoch.
    pub fn call_values(&self, rustia: &mut Rustia, args: &[LiveValue]) -> Result<Vec<LiveValue>> {
        let function = self.value.raw(rustia.vm(), "function handle")?;
        let raw_args = args
            .iter()
            .map(|value| value.raw(rustia.vm(), "function argument handle"))
            .collect::<Result<Vec<_>>>()?;
        let results = rustia
            .vm_mut()
            .call_retained_function(function, &raw_args)
            .map_err(Error::from_engine)?;
        LiveValue::retain_many(rustia.vm_mut(), &results)
    }

    /// Calls the function within a typed host invocation's shared budget.
    ///
    /// If argument conversion, execution, or result conversion fails, records
    /// staged by this call are discarded. Guest table writes and external host
    /// changes are not rolled back by this operation.
    /// As with [`call`](Self::call), every returned value must fit [`OwnedValue`]
    /// before the requested return type selects or discards results.
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

/// The live value returned by a successfully installed script.
///
/// Scripts commonly return a table of named functions and data. Use
/// [`function`](Self::function) to retain a callable export, [`get`](Self::get)
/// to copy data into Rust, or [`get_live`](Self::get_live) to keep a live object.
/// The exports remain usable after another module is installed, but all clones
/// become invalid after [`Rustia::reset_decision`].
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

    /// Clones the underlying live value, whether or not it is a table.
    pub fn as_value(&self) -> LiveValue {
        self.value.clone()
    }

    /// Returns a live table handle, or `None` if the installed script returned another type.
    pub fn as_table<C>(&self, rustia: &Rustia<C>) -> Result<Option<crate::Table>>
    where
        C: HostSchema,
    {
        self.value.as_table(rustia)
    }

    /// Returns the decision epoch in which these exports were retained.
    pub fn decision_epoch(&self) -> u64 {
        self.decision_epoch
    }

    /// Copies the entire returned value into Rust-owned data.
    ///
    /// A returned table containing functions or other values unsupported by
    /// [`OwnedValue`] cannot be copied as a whole; read individual exports instead.
    pub fn value(&self, rustia: &mut Rustia) -> Result<OwnedValue> {
        let value = self.value.raw(rustia.vm(), "exports handle")?;
        snapshot_value(rustia.vm_mut(), value)
    }

    /// Copies the returned value within a typed host invocation.
    pub fn value_in<C>(&self, invocation: &mut Invocation<'_, C>) -> Result<OwnedValue>
    where
        C: HostSchema,
    {
        invocation.scoped(|invocation| {
            let value = self.value.raw(invocation.vm, "exports handle")?;
            snapshot_value(invocation.vm, value)
        })
    }

    /// Deserializes the returned value without first building an [`OwnedValue`] copy.
    pub fn deserialize<T>(&self, rustia: &mut Rustia<impl HostSchema>) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let value = self.value.raw(rustia.vm(), "exports handle")?;
        from_live(rustia.vm_mut(), value)
    }

    /// Reads and converts a string-keyed export without invoking `__index`.
    ///
    /// A missing key is `nil`, so it can be read as `Option<T>`.
    pub fn get<T>(&self, rustia: &mut Rustia, name: &str) -> Result<T>
    where
        T: FromRustia,
    {
        let value = self.raw_get(rustia, name)?;
        let value = snapshot_value(rustia.vm_mut(), value)?;
        T::from_rustia(value)
    }

    /// Reads a string-keyed export as a live value without invoking `__index`.
    pub fn get_live(&self, rustia: &mut Rustia, name: &str) -> Result<LiveValue> {
        let value = self.raw_get(rustia, name)?;
        LiveValue::retain(rustia.vm_mut(), value)
    }

    /// Reads and converts an export within a typed host invocation.
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

    /// Reads a live export within a typed host invocation.
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

    /// Retains a named function export, or returns an error if it is not a function.
    ///
    /// Lookup uses the table's stored value and does not invoke `__index`.
    pub fn function(&self, rustia: &mut Rustia, name: &str) -> Result<Function> {
        let value = self.raw_get(rustia, name)?;
        if value.kind() != ValueKind::Function {
            return Err(Error::embed(format!("export `{name}` is not a function")));
        }
        let value = rustia
            .vm_mut()
            .retain_value(value)
            .map_err(Error::from_engine)?;
        Ok(Function::new(value, self.decision_epoch))
    }

    /// Retains a named function export within a typed host invocation.
    pub fn function_in<C>(&self, invocation: &mut Invocation<'_, C>, name: &str) -> Result<Function>
    where
        C: HostSchema,
    {
        invocation.scoped(|invocation| {
            let value = self.raw_get_in(invocation, name)?;
            if value.kind() != ValueKind::Function {
                return Err(Error::embed(format!("export `{name}` is not a function")));
            }
            let value = invocation
                .vm
                .retain_value(value)
                .map_err(Error::from_engine)?;
            Ok(Function::new(value, self.decision_epoch))
        })
    }

    fn raw_get(&self, rustia: &mut Rustia, name: &str) -> Result<RawValue> {
        let table = self.value.raw(rustia.vm(), "exports handle")?;
        let key = rustia.vm_mut().string(name);
        rustia
            .vm_mut()
            .raw_table_get_value(table, key)
            .map_err(Error::from_engine)
    }

    fn raw_get_in<C>(&self, invocation: &mut Invocation<'_, C>, name: &str) -> Result<RawValue>
    where
        C: HostSchema,
    {
        let table = self.value.raw(invocation.vm, "exports handle")?;
        let key = invocation.vm.string(name);
        invocation
            .vm
            .raw_table_get_value(table, key)
            .map_err(Error::from_engine)
    }
}

// Internal adapters carry an unconstructible token because methods of a private
// supertrait can otherwise be called through a public generic marker bound.
pub(crate) mod adapter {
    use super::*;

    pub(crate) struct Token;

    pub(crate) trait Function<Args, Return>: Send + Sync + 'static {
        fn into_host_function(
            self,
            token: Token,
            name: String,
            fuel_cost: u32,
            capability: OpCapability,
        ) -> HostFunctionOp;
    }

    pub(crate) trait Value {
        fn into_host_value(self, token: Token) -> Result<HostReturnValue>;
    }

    pub(crate) trait Argument: Sized {
        fn from_host_argument(token: Token, vm: &mut Vm, value: RawValue) -> Result<Self>;
    }

    pub(crate) trait Return {
        fn into_host_return(self, token: Token) -> Result<Vec<HostReturnValue>>;

        fn rustia_type_names(token: Token) -> Vec<&'static str>
        where
            Self: Sized;

        fn is_fallible(_token: Token) -> bool
        where
            Self: Sized,
        {
            false
        }
    }
}

use adapter::Value as _;

/// A synchronous Rust callback accepted by [`crate::HostFunction`].
///
/// Closures and function pointers with up to eight arguments implement this
/// trait automatically. It can be used as a generic bound but cannot be
/// implemented directly; use [`FromRustia`] and [`IntoRustia`] for custom data
/// conversions.
///
/// ```compile_fail
/// struct CustomAdapter;
/// impl rustia::IntoHostFunction<(), ()> for CustomAdapter {}
/// ```
#[allow(private_bounds)]
pub trait IntoHostFunction<Args, Return>:
    adapter::Function<Args, Return> + Send + Sync + 'static
{
}

impl<T, Args, Return> IntoHostFunction<Args, Return> for T where T: adapter::Function<Args, Return> {}

pub(crate) enum HostReturnValue {
    Owned(OwnedValue),
    Live(LiveValue),
}

/// One callback result supported by Rustia's built-in callback adapters.
///
/// Implement [`IntoRustia`] for custom owned data. Live [`crate::Value`],
/// [`crate::Table`], and [`Function`] handles also implement this sealed trait.
#[allow(private_bounds)]
pub trait IntoHostValue: adapter::Value {}

impl<T: adapter::Value> IntoHostValue for T {}

impl<T: IntoRustia> adapter::Value for T {
    fn into_host_value(self, _token: adapter::Token) -> Result<HostReturnValue> {
        self.into_rustia().map(HostReturnValue::Owned)
    }
}

impl adapter::Value for LiveValue {
    fn into_host_value(self, _token: adapter::Token) -> Result<HostReturnValue> {
        Ok(HostReturnValue::Live(self))
    }
}

impl adapter::Value for crate::Table {
    fn into_host_value(self, token: adapter::Token) -> Result<HostReturnValue> {
        self.as_value().into_host_value(token)
    }
}

impl adapter::Value for Function {
    fn into_host_value(self, token: adapter::Token) -> Result<HostReturnValue> {
        self.as_value().into_host_value(token)
    }
}

/// An owned or live argument accepted by Rustia's built-in callback adapters.
///
/// Implement [`FromRustia`] for custom owned arguments. Live [`crate::Value`],
/// [`crate::Table`], and [`Function`] handles are accepted directly. This trait
/// is sealed and can be used as a generic bound.
///
/// ```compile_fail
/// struct CustomArgument;
/// impl rustia::FromHostArgument for CustomArgument {}
/// ```
#[allow(private_bounds)]
pub trait FromHostArgument: adapter::Argument + Sized {}

impl<T: adapter::Argument> FromHostArgument for T {}

impl<T: FromRustia> adapter::Argument for T {
    fn from_host_argument(_token: adapter::Token, vm: &mut Vm, value: RawValue) -> Result<Self> {
        T::from_rustia(snapshot_value(vm, value)?)
    }
}

impl adapter::Argument for LiveValue {
    fn from_host_argument(_token: adapter::Token, vm: &mut Vm, value: RawValue) -> Result<Self> {
        LiveValue::retain(vm, value)
    }
}

impl adapter::Argument for crate::Table {
    fn from_host_argument(_token: adapter::Token, vm: &mut Vm, value: RawValue) -> Result<Self> {
        if value.kind() != ValueKind::Table {
            return Err(Error::embed("expected table".to_owned()));
        }
        Ok(Self::new(LiveValue::retain(vm, value)?))
    }
}

impl adapter::Argument for Function {
    fn from_host_argument(_token: adapter::Token, vm: &mut Vm, value: RawValue) -> Result<Self> {
        if value.kind() != ValueKind::Function {
            return Err(Error::embed("expected function".to_owned()));
        }
        Ok(Self::from_value(LiveValue::retain(vm, value)?))
    }
}

/// A complete return value accepted by Rustia's built-in callback adapters.
///
/// `()` returns no results; tuples through eight elements return separate
/// results and can mix owned data with live handles. [`Result`] reports a
/// callback failure or converts its successful value by the same rules.
/// This trait is sealed; custom owned results use [`IntoRustia`] and can be
/// returned as a one-element tuple.
///
/// ```compile_fail
/// struct CustomReturn;
/// impl rustia::IntoHostReturn for CustomReturn {}
/// ```
#[allow(private_bounds)]
pub trait IntoHostReturn: adapter::Return {}

impl<T: adapter::Return> IntoHostReturn for T {}

macro_rules! impl_single_host_return {
    ($($ty:ty),+ $(,)?) => {$(
        impl adapter::Return for $ty {
            fn into_host_return(self, _token: adapter::Token) -> Result<Vec<HostReturnValue>> {
                Ok(vec![self.into_host_value(adapter::Token)?])
            }
            fn rustia_type_names(_token: adapter::Token) -> Vec<&'static str> {
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
        impl<T: IntoRustia> adapter::Return for $ty {
            fn into_host_return(self, _token: adapter::Token) -> Result<Vec<HostReturnValue>> {
                Ok(vec![self.into_host_value(adapter::Token)?])
            }
            fn rustia_type_names(_token: adapter::Token) -> Vec<&'static str> {
                vec![type_name::<Self>()]
            }
        }
    };
}

impl_owned_container_host_return!(Option<T>);
impl_owned_container_host_return!(Vec<T>);
impl_owned_container_host_return!(std::collections::BTreeMap<String, T>);

impl adapter::Return for () {
    fn into_host_return(self, _token: adapter::Token) -> Result<Vec<HostReturnValue>> {
        Ok(Vec::new())
    }
    fn rustia_type_names(_token: adapter::Token) -> Vec<&'static str> {
        Vec::new()
    }
}

macro_rules! impl_tuple_host_return {
    ($($name:ident),+ $(,)?) => {
        impl<$($name: IntoHostValue),+> adapter::Return for ($($name,)+) {
            #[allow(non_snake_case)]
            fn into_host_return(self, _token: adapter::Token) -> Result<Vec<HostReturnValue>> {
                let ($($name,)+) = self;
                Ok(vec![$($name.into_host_value(adapter::Token)?,)+])
            }
            fn rustia_type_names(_token: adapter::Token) -> Vec<&'static str> {
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

impl<T: IntoHostReturn> adapter::Return for Result<T> {
    fn into_host_return(self, _token: adapter::Token) -> Result<Vec<HostReturnValue>> {
        self.and_then(|value| value.into_host_return(adapter::Token))
    }
    fn rustia_type_names(_token: adapter::Token) -> Vec<&'static str> {
        T::rustia_type_names(adapter::Token)
    }
    fn is_fallible(_token: adapter::Token) -> bool {
        true
    }
}

impl<F, Return> adapter::Function<(), Return> for F
where
    F: Fn() -> Return + Send + Sync + 'static,
    Return: IntoHostReturn,
{
    fn into_host_function(
        self,
        _token: adapter::Token,
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
            Return::rustia_type_names(adapter::Token),
            Return::is_fallible(adapter::Token),
        )
    }
}

macro_rules! impl_host_function {
    ($parameter_count:expr; $($name:ident),+ $(,)?) => {
        impl<Func, Return, $($name),+> adapter::Function<($($name,)+), Return> for Func
        where
            Func: Fn($($name),+) -> Return + Send + Sync + 'static,
            Return: IntoHostReturn,
            $($name: FromHostArgument,)+
        {
            #[allow(non_snake_case)]
            fn into_host_function(
                self,
                _token: adapter::Token,
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
                    Return::rustia_type_names(adapter::Token),
                    Return::is_fallible(adapter::Token),
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
    T::from_host_argument(adapter::Token, vm, value)
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
        .into_host_return(adapter::Token)
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
        } => (*error).into_core(),
        error => embed_to_vm_error(
            error.with_context(format!("host function `{function_name}` {}", slot.as_ref())),
        ),
    }
}
