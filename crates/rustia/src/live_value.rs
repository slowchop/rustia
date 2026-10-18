use crate::function::Function;
use crate::serde_value::from_live;
use crate::value::{load_owned_value, snapshot_value};
use crate::{Error, HostSchema, IntoRustia, OwnedValue, Result, Rustia};
use rustia_vm::{RetainedValue, Value as RawValue, ValueKind, Vm};
use std::fmt;

/// The Luau type of a live [`Value`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ValueType {
    Nil,
    Boolean,
    Number,
    String,
    Table,
    Function,
    Userdata,
    Buffer,
    Vector,
}

impl ValueType {
    /// Returns the Luau runtime type name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Nil => "nil",
            Self::Boolean => "boolean",
            Self::Number => "number",
            Self::String => "string",
            Self::Table => "table",
            Self::Function => "function",
            Self::Userdata => "userdata",
            Self::Buffer => "buffer",
            Self::Vector => "vector",
        }
    }
}

/// A script value kept alive in its original Rustia VM.
///
/// Unlike [`OwnedValue`], this is a handle rather than a copy. Tables, functions,
/// and userdata keep their identity; strings can contain arbitrary bytes; tables
/// can share values or refer to themselves. Cloning a handle keeps the same
/// value alive and does not copy its contents.
///
/// Every operation requires the VM that created the handle. A
/// [`Rustia::reset_decision`] invalidates all earlier handles; using one after
/// reset or with another VM returns an error. To keep ordinary data across a
/// reset, copy it with [`to_owned`](Self::to_owned) or
/// [`deserialize`](Self::deserialize).
#[derive(Clone)]
pub struct Value {
    value: RetainedValue,
    decision_epoch: u64,
}

impl Value {
    pub(crate) fn new(value: RetainedValue, decision_epoch: u64) -> Self {
        Self {
            value,
            decision_epoch,
        }
    }

    pub(crate) fn retain(vm: &mut Vm, value: RawValue) -> Result<Self> {
        let decision_epoch = vm.decision_epoch();
        vm.retain_value(value)
            .map(|value| Self::new(value, decision_epoch))
            .map_err(Error::from)
    }

    pub(crate) fn retain_many(vm: &mut Vm, values: &[RawValue]) -> Result<Vec<Self>> {
        let decision_epoch = vm.decision_epoch();
        vm.retain_values(values)
            .map(|values| {
                values
                    .into_iter()
                    .map(|value| Self::new(value, decision_epoch))
                    .collect()
            })
            .map_err(Error::from)
    }

    /// Returns the decision epoch in which this value was retained.
    pub const fn decision_epoch(&self) -> u64 {
        self.decision_epoch
    }

    /// Returns this value's runtime kind.
    pub fn kind<C>(&self, rustia: &Rustia<C>) -> Result<ValueType>
    where
        C: HostSchema,
    {
        let value = self.raw(rustia.vm(), "value handle")?;
        raw_value_type(value)
    }

    /// Returns this value's Luau runtime type name.
    pub fn type_name<C>(&self, rustia: &Rustia<C>) -> Result<&'static str>
    where
        C: HostSchema,
    {
        self.kind(rustia).map(ValueType::name)
    }

    /// Returns whether this is the nil value.
    pub fn is_nil<C>(&self, rustia: &Rustia<C>) -> Result<bool>
    where
        C: HostSchema,
    {
        self.kind(rustia).map(|kind| kind == ValueType::Nil)
    }

    /// Tests Luau truthiness: only `nil` and `false` are false.
    /// This does not invoke guest code.
    pub fn truthy<C>(&self, rustia: &Rustia<C>) -> Result<bool>
    where
        C: HostSchema,
    {
        self.raw(rustia.vm(), "value handle").map(RawValue::truthy)
    }

    /// Returns a boolean, or `None` for another value kind. No coercion is applied.
    pub fn as_boolean<C>(&self, rustia: &Rustia<C>) -> Result<Option<bool>>
    where
        C: HostSchema,
    {
        self.raw(rustia.vm(), "value handle").map(RawValue::as_bool)
    }

    /// Returns a number, or `None` for another value kind. No coercion is applied.
    pub fn as_number<C>(&self, rustia: &Rustia<C>) -> Result<Option<f64>>
    where
        C: HostSchema,
    {
        self.raw(rustia.vm(), "value handle")
            .map(RawValue::as_number)
    }

    /// Copies the exact binary string bytes, or returns `None` for another kind.
    pub fn string_bytes<C>(&self, rustia: &mut Rustia<C>) -> Result<Option<Vec<u8>>>
    where
        C: HostSchema,
    {
        let value = self.raw(rustia.vm(), "value handle")?;
        rustia
            .vm_mut()
            .string_value_bytes(value)
            .map_err(Error::from)
    }

    /// Copies the current buffer bytes, or returns `None` for another kind.
    pub fn buffer_bytes<C>(&self, rustia: &mut Rustia<C>) -> Result<Option<Vec<u8>>>
    where
        C: HostSchema,
    {
        let value = self.raw(rustia.vm(), "value handle")?;
        rustia
            .vm_mut()
            .buffer_value_bytes(value)
            .map_err(Error::from)
    }

    /// Returns the three vector components, or `None` for another value kind.
    pub fn vector_components<C>(&self, rustia: &mut Rustia<C>) -> Result<Option<[f32; 3]>>
    where
        C: HostSchema,
    {
        let value = self.raw(rustia.vm(), "value handle")?;
        rustia
            .vm_mut()
            .vector_value_components(value)
            .map_err(Error::from)
    }

    /// Copies this value and any nested tables into Rust-owned data.
    ///
    /// Non-UTF-8 strings, buffers, vectors, userdata, functions, keys unsupported
    /// by [`crate::OwnedKey`], and cyclic graphs return conversion errors.
    /// Shared tables are copied independently; table identity is not preserved.
    /// Deeply nested tables also return an error once the snapshot depth limit
    /// is reached.
    pub fn to_owned<C>(&self, rustia: &mut Rustia<C>) -> Result<OwnedValue>
    where
        C: HostSchema,
    {
        let value = self.raw(rustia.vm(), "value handle")?;
        snapshot_value(rustia.vm_mut(), value)
    }

    /// Deserializes this value without first building an [`OwnedValue`] copy.
    ///
    /// The returned type must own its data; it cannot borrow strings or other
    /// contents from the VM.
    ///
    /// ```compile_fail
    /// use rustia::{Rustia, Value};
    ///
    /// fn cannot_borrow(value: &Value, rustia: &mut Rustia) {
    ///     let _: &str = value.deserialize(rustia).unwrap();
    /// }
    /// ```
    pub fn deserialize<T>(&self, rustia: &mut Rustia<impl HostSchema>) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let value = self.raw(rustia.vm(), "value handle")?;
        from_live(rustia.vm_mut(), value)
    }

    /// Returns a table handle, or `None` if this is another value kind.
    pub fn as_table<C>(&self, rustia: &Rustia<C>) -> Result<Option<Table>>
    where
        C: HostSchema,
    {
        Ok((self.kind(rustia)? == ValueType::Table).then(|| Table::new(self.clone())))
    }

    /// Returns a function handle, or `None` if this is another value kind.
    pub fn as_function<C>(&self, rustia: &Rustia<C>) -> Result<Option<Function>>
    where
        C: HostSchema,
    {
        Ok((self.kind(rustia)? == ValueType::Function).then(|| Function::from_value(self.clone())))
    }

    /// Compares values using raw Luau equality without invoking `__eq`.
    pub fn raw_equal<C>(&self, rustia: &mut Rustia<C>, other: &Self) -> Result<bool>
    where
        C: HostSchema,
    {
        let lhs = self.raw(rustia.vm(), "left value handle")?;
        let rhs = other.raw(rustia.vm(), "right value handle")?;
        rustia
            .vm_mut()
            .raw_equal_values(lhs, rhs)
            .map_err(Error::from)
    }

    pub(crate) fn raw(&self, vm: &Vm, label: &str) -> Result<RawValue> {
        self.value
            .value(vm)
            .map_err(|error| Error::from(error).with_context(label))
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Value")
            .field("decision_epoch", &self.decision_epoch)
            .finish_non_exhaustive()
    }
}

/// A live table that Rust and Luau can both read and change.
///
/// Clones refer to the same table. All lookups and writes are raw: they bypass
/// `__index` and `__newindex`. Like [`Value`], a table belongs to one VM and
/// becomes invalid after [`Rustia::reset_decision`].
///
/// ```
/// use rustia::Rustia;
///
/// let mut lua = Rustia::builder().build()?;
/// let scores = lua.create_table()?;
/// scores.set(&mut lua, "Ada", 42)?;
/// assert_eq!(scores.get(&mut lua, "Ada")?.as_number(&lua)?, Some(42.0));
/// # Ok::<(), rustia::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct Table {
    value: Value,
}

impl Table {
    pub(crate) const fn new(value: Value) -> Self {
        Self { value }
    }

    pub(crate) fn raw(&self, vm: &Vm, label: &str) -> Result<RawValue> {
        self.value.raw(vm, label)
    }

    /// Clones this table's underlying live value handle.
    pub fn as_value(&self) -> Value {
        self.value.clone()
    }

    /// Deserializes this table without first building an [`OwnedValue`] copy.
    pub fn deserialize<T>(&self, rustia: &mut Rustia<impl HostSchema>) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        self.value.deserialize(rustia)
    }

    /// Returns the decision epoch in which this handle was retained.
    pub const fn decision_epoch(&self) -> u64 {
        self.value.decision_epoch()
    }

    /// Looks up a live key without invoking `__index`. Missing keys return `nil`.
    pub fn get_value<C>(&self, rustia: &mut Rustia<C>, key: &Value) -> Result<Value>
    where
        C: HostSchema,
    {
        let table = self.value.raw(rustia.vm(), "table handle")?;
        let key = key.raw(rustia.vm(), "table key handle")?;
        let value = rustia
            .vm_mut()
            .raw_table_get_value(table, key)
            .map_err(Error::from)?;
        Value::retain(rustia.vm_mut(), value)
    }

    /// Looks up a Rust key without invoking `__index`. Missing keys return `nil`.
    pub fn get<K, C>(&self, rustia: &mut Rustia<C>, key: K) -> Result<Value>
    where
        K: IntoRustia,
        C: HostSchema,
    {
        let table = self.value.raw(rustia.vm(), "table handle")?;
        let key = key.into_rustia()?;
        let key = load_owned_value(rustia.vm_mut(), &key)?;
        let value = rustia
            .vm_mut()
            .raw_table_get_value(table, key)
            .map_err(Error::from)?;
        Value::retain(rustia.vm_mut(), value)
    }

    /// Performs a raw write with live key/value handles, bypassing `__newindex`.
    pub fn set_value<C>(&self, rustia: &mut Rustia<C>, key: &Value, value: &Value) -> Result<()>
    where
        C: HostSchema,
    {
        let table = self.value.raw(rustia.vm(), "table handle")?;
        let key = key.raw(rustia.vm(), "table key handle")?;
        let value = value.raw(rustia.vm(), "table value handle")?;
        rustia
            .vm_mut()
            .raw_table_set_value(table, key, value)
            .map_err(Error::from)
    }

    /// Stores a live value under a Rust key without invoking `__newindex`.
    ///
    /// The stored value keeps its identity; it is not copied into a new object.
    pub fn set_live<K, C>(&self, rustia: &mut Rustia<C>, key: K, value: &Value) -> Result<()>
    where
        K: IntoRustia,
        C: HostSchema,
    {
        // Validate both live handles before key conversion can allocate or
        // mutate VM-owned storage. The retained handles keep these compact raw
        // values rooted if loading the owned key triggers quota collection.
        let table = self.value.raw(rustia.vm(), "table handle")?;
        let value = value.raw(rustia.vm(), "table value handle")?;
        let key = key.into_rustia()?;
        let key = load_owned_value(rustia.vm_mut(), &key)?;
        rustia
            .vm_mut()
            .raw_table_set_value(table, key, value)
            .map_err(Error::from)
    }

    /// Converts and stores a Rust key and value without invoking `__newindex`.
    ///
    /// Use [`set_live`](Self::set_live) to store an existing script object.
    pub fn set<K, V, C>(&self, rustia: &mut Rustia<C>, key: K, value: V) -> Result<()>
    where
        K: IntoRustia,
        V: IntoRustia,
        C: HostSchema,
    {
        let table = self.value.raw(rustia.vm(), "table handle")?;
        let key = key.into_rustia()?;
        let value = value.into_rustia()?;
        let key = load_owned_value(rustia.vm_mut(), &key)?;
        let value = load_owned_value(rustia.vm_mut(), &value)?;
        rustia
            .vm_mut()
            .raw_table_set_value(table, key, value)
            .map_err(Error::from)
    }

    /// Returns the current entries in insertion order as live key/value handles.
    ///
    /// Mutating the table later does not change which key/value handles this
    /// returned vector contains, though those handles still refer to their
    /// original live objects.
    pub fn entries<C>(&self, rustia: &mut Rustia<C>) -> Result<Vec<(Value, Value)>>
    where
        C: HostSchema,
    {
        let table = self.value.raw(rustia.vm(), "table handle")?;
        let entries = rustia
            .vm_mut()
            .raw_table_entries_value(table)
            .map_err(Error::from)?;
        let raw_values = entries
            .into_iter()
            .flat_map(|(key, value)| [key, value])
            .collect::<Vec<_>>();
        let values = Value::retain_many(rustia.vm_mut(), &raw_values)?;
        Ok(values
            .chunks_exact(2)
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect())
    }
}

fn raw_value_type(value: RawValue) -> Result<ValueType> {
    Ok(match value.kind() {
        ValueKind::Nil => ValueType::Nil,
        ValueKind::Boolean => ValueType::Boolean,
        ValueKind::Number => ValueType::Number,
        ValueKind::String => ValueType::String,
        ValueKind::Table => ValueType::Table,
        ValueKind::Function => ValueType::Function,
        ValueKind::Userdata => ValueType::Userdata,
        ValueKind::Buffer => ValueType::Buffer,
        ValueKind::Vector => ValueType::Vector,
        _ => {
            return Err(Error::embed(
                "unknown VM value kind cannot be represented as a live value".to_owned(),
            ));
        }
    })
}
