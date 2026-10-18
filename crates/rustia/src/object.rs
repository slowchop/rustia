use crate::function::adapter::Token;
use crate::object_method::{IntoMutObjectMethod, IntoObjectMethod};
use crate::value::{load_owned_arguments, snapshot_value};
use crate::{
    Error, FromRustiaMulti, HostSchema, IntoRustiaMulti, Invocation, OpCapability, Result, Rustia,
    Value,
};
use rustia_vm::{HostFunctionOp, HostObjectTypeDefinition, Value as RawValue};
use std::any::{type_name, TypeId};
use std::marker::PhantomData;

/// A Rust type and the methods scripts may call on it.
///
/// Register the definition once, then create as many objects as needed. Each
/// object keeps the method definition it was created with. Re-registering a
/// type changes future objects; existing objects and detached methods keep
/// their original behavior. Definitions survive decision reset.
///
/// ```
/// use rustia::{ObjectType, Rustia};
/// struct Player { score: i64 }
/// let mut lua = Rustia::builder().build()?;
/// lua.register_object_type(ObjectType::<Player>::new("Player")
///     .method("score", 1, |player: &Player| player.score))?;
/// let player = lua.create_object(Player { score: 42 })?;
/// lua.globals()?.set_live(&mut lua, "player", &player.as_value())?;
/// assert_eq!(lua.eval::<i64>("return player:score()")?, 42);
/// # Ok::<(), rustia::Error>(())
/// ```
///
/// Read-only methods promise replay-stable input, including anything reached
/// through captures or interior mutability. Use capability-specific methods
/// for staged actions or external changes. Opaque Rust payload allocations
/// are outside the VM memory limit, like callback captures. Payload destructors
/// must not panic; the application must avoid ownership cycles in Rust data.
pub struct ObjectType<T: 'static, H: HostSchema = ()> {
    name: String,
    pub(crate) methods: Vec<HostFunctionOp>,
    marker: PhantomData<fn() -> (T, H)>,
}

impl<T: 'static, H: HostSchema> ObjectType<T, H> {
    /// Starts a method definition for `T`, using `name` in diagnostics.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            methods: Vec::new(),
            marker: PhantomData,
        }
    }

    /// Adds a read-only method with an explicit host-call fuel cost.
    ///
    /// The receiver comes first, followed by up to eight ordinary or live
    /// arguments. The callback follows the same return and panic rules as
    /// [`crate::HostFunction`]. It receives no VM borrow and cannot re-enter Luau.
    pub fn method<F, A, R>(self, name: impl Into<String>, fuel_cost: u32, method: F) -> Self
    where
        F: IntoObjectMethod<T, A, R>,
    {
        self.method_with_capability(name, fuel_cost, OpCapability::Pure, method)
    }

    /// Adds a read-only method with a declared captured-state capability.
    ///
    /// A read-only Rust receiver does not make captured or interior state
    /// deterministic. Declare its actual capability here, or use the typed
    /// context helpers when borrowing per-invocation application state.
    pub fn method_with_capability<F, A, R>(
        mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        capability: OpCapability,
        method: F,
    ) -> Self
    where
        F: IntoObjectMethod<T, A, R>,
    {
        self.methods.push(method.into_object_method(
            Token,
            name.into(),
            fuel_cost,
            capability.to_core(),
        ));
        self
    }

    /// Adds a method that changes its Rust payload immediately.
    ///
    /// This declares external mutable state and is rejected atomically by
    /// deterministic construction. Payload changes survive script errors and
    /// failed installation; reset drops the payload. Effects on external
    /// application state are not undone by reset.
    pub fn method_mut<F, A, R>(self, name: impl Into<String>, fuel_cost: u32, method: F) -> Self
    where
        F: IntoMutObjectMethod<T, A, R>,
    {
        self.mut_method(
            name.into(),
            fuel_cost,
            OpCapability::ReadsExternalMutableState,
            method,
        )
    }

    /// Adds an immediately mutable method that also performs nondeterministic effects.
    pub fn method_mut_nondeterministic<F, A, R>(
        self,
        name: impl Into<String>,
        fuel_cost: u32,
        method: F,
    ) -> Self
    where
        F: IntoMutObjectMethod<T, A, R>,
    {
        self.mut_method(
            name.into(),
            fuel_cost,
            OpCapability::NondeterministicEffect,
            method,
        )
    }

    fn mut_method<F, A, R>(
        mut self,
        name: String,
        fuel_cost: u32,
        capability: OpCapability,
        method: F,
    ) -> Self
    where
        F: IntoMutObjectMethod<T, A, R>,
    {
        self.methods.push(method.into_mut_object_method(
            Token,
            name,
            fuel_cost,
            capability.to_core(),
        ));
        self
    }
}

/// A live, type-checked handle to a Rust object in one VM and decision epoch.
///
/// Clones share the same object without cloning `T`. Payload references exist
/// only during method callbacks. Reset invalidates every handle and releases
/// the VM's payload ownership even if an old Rust handle is still retained.
pub struct Object<T: 'static> {
    value: Value,
    marker: PhantomData<fn() -> T>,
}

impl<T: 'static> Clone for Object<T> {
    fn clone(&self) -> Self {
        Self::new(self.value.clone())
    }
}

impl<T: 'static> std::fmt::Debug for Object<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Object")
            .field("rust_type", &type_name::<T>())
            .field("value", &self.value)
            .finish()
    }
}

impl<T: 'static> Object<T> {
    pub(crate) fn new(value: Value) -> Self {
        Self {
            value,
            marker: PhantomData,
        }
    }

    /// Clones the underlying live value for globals, tables, and callback results.
    pub fn as_value(&self) -> Value {
        self.value.clone()
    }

    /// Returns the decision epoch in which this object was retained.
    pub fn decision_epoch(&self) -> u64 {
        self.value.decision_epoch()
    }

    fn receiver(&self, vm: &mut rustia_vm::Vm, name: &str) -> Result<(RawValue, RawValue)> {
        let receiver = self.value.raw(vm, "object handle")?;
        let method = vm
            .host_object_method(receiver, name)
            .map_err(Error::from_engine)?;
        Ok((receiver, method))
    }

    /// Calls a method with Rust arguments and copies all returned values into Rust.
    ///
    /// The receiver is supplied automatically. `()` passes no arguments. Use
    /// [`call_method_values`](Self::call_method_values) for object results or
    /// other values that cannot be copied into [`crate::OwnedValue`].
    pub fn call_method<A: IntoRustiaMulti, R: FromRustiaMulti>(
        &self,
        lua: &mut Rustia,
        name: &str,
        args: A,
    ) -> Result<R> {
        let (receiver, method) = self.receiver(lua.vm_mut(), name)?;
        let owned = args.into_rustia_multi()?;
        let (mut raw, _argument_roots) = load_owned_arguments(lua.vm_mut(), &owned)?;
        raw.insert(0, receiver);
        let results = lua
            .vm_mut()
            .call_retained_function(method, &raw)
            .map_err(Error::from_engine)?;
        R::from_rustia_multi(
            results
                .into_iter()
                .map(|v| snapshot_value(lua.vm_mut(), v))
                .collect::<Result<_>>()?,
        )
    }

    /// Calls a method with live arguments and keeps every result live.
    pub fn call_method_values(
        &self,
        lua: &mut Rustia,
        name: &str,
        args: &[Value],
    ) -> Result<Vec<Value>> {
        let (receiver, method) = self.receiver(lua.vm_mut(), name)?;
        let mut raw = vec![receiver];
        for value in args {
            raw.push(value.raw(lua.vm(), "object method argument")?);
        }
        let results = lua
            .vm_mut()
            .call_retained_function(method, &raw)
            .map_err(Error::from_engine)?;
        Value::retain_many(lua.vm_mut(), &results)
    }

    /// Calls a method within a typed invocation's shared budget and record journal.
    ///
    /// A conversion or execution error discards this call's staged records.
    /// Guest writes and external application changes remain.
    pub fn call_method_in<H: HostSchema, A: IntoRustiaMulti, R: FromRustiaMulti>(
        &self,
        invocation: &mut Invocation<'_, H>,
        name: &str,
        args: A,
    ) -> Result<R> {
        invocation.scoped(|invocation| {
            let (receiver, method) = self.receiver(invocation.vm, name)?;
            let owned = args.into_rustia_multi()?;
            let (mut raw, _argument_roots) = load_owned_arguments(invocation.vm, &owned)?;
            raw.insert(0, receiver);
            let results = invocation.call_retained_raw(method, &raw)?;
            R::from_rustia_multi(
                results
                    .into_iter()
                    .map(|v| snapshot_value(invocation.vm, v))
                    .collect::<Result<_>>()?,
            )
        })
    }

    /// Calls a method with live values inside a typed invocation.
    pub fn call_method_values_in<H: HostSchema>(
        &self,
        invocation: &mut Invocation<'_, H>,
        name: &str,
        args: &[Value],
    ) -> Result<Vec<Value>> {
        invocation.scoped(|invocation| {
            let (receiver, method) = self.receiver(invocation.vm, name)?;
            let mut raw = vec![receiver];
            for value in args {
                raw.push(value.raw(invocation.vm, "object method argument")?);
            }
            let results = invocation.call_retained_raw(method, &raw)?;
            Value::retain_many(invocation.vm, &results)
        })
    }
}

/// A fresh Rust payload returned by a synchronous callback.
///
/// Register `T` before the callback runs. Rustia checks every fresh-object
/// registration and live result before allocating any guest return values.
/// This can be mixed with ordinary and live results in a tuple.
pub struct NewObject<T: 'static> {
    pub(crate) value: T,
}

impl<T: 'static> NewObject<T> {
    /// Moves `value` into a new guest object when the callback returns.
    pub fn new(value: T) -> Self {
        Self { value }
    }
}

impl<H: HostSchema> Rustia<H> {
    /// Registers or replaces the methods used by future objects of `T`.
    ///
    /// The complete definition is validated before publication. A forbidden
    /// method rejects the whole definition, preserving any previous one.
    pub fn register_object_type<T: 'static>(&mut self, definition: ObjectType<T, H>) -> Result<()> {
        self.vm_mut()
            .register_host_object_type(HostObjectTypeDefinition {
                rust_type_id: TypeId::of::<T>(),
                rust_type_name: type_name::<T>(),
                name: definition.name,
                methods: definition.methods,
            })
            .map_err(Error::from_engine)
    }

    /// Moves a payload into a registered object type and retains its live handle.
    ///
    /// `T` need not implement `Clone`, `Send`, or `Sync`. Hardened leaves reject
    /// object construction before allocating guest memory.
    pub fn create_object<T: 'static>(&mut self, value: T) -> Result<Object<T>> {
        let epoch = self.vm().decision_epoch();
        let value = self
            .vm_mut()
            .create_retained_host_object(value)
            .map_err(Error::from_engine)?;
        Ok(Object::new(Value::new(value, epoch)))
    }
}

impl Value {
    /// Returns a typed Rust-object handle, or `None` for another payload type.
    ///
    /// A stale handle or a handle from another VM returns an error first.
    pub fn as_object<T: 'static>(
        &self,
        lua: &mut Rustia<impl HostSchema>,
    ) -> Result<Option<Object<T>>> {
        let raw = self.raw(lua.vm(), "value handle")?;
        Ok(
            (lua.vm_mut().host_object_type_id(raw) == Some(TypeId::of::<T>()))
                .then(|| Object::new(self.clone())),
        )
    }
}
