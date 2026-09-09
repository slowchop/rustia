use crate::function::Function;
use crate::serde_value::from_live;
use crate::value::{load_owned_value, snapshot_value};
use crate::{Error, HostSchema, IntoRustia, OwnedValue, Result, Rustia};
use rustia_vm::{RetainedValue, Value as RawValue, ValueKind, Vm};
use std::fmt;

/// The runtime kind of a rooted live script value.
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

/// A live script value rooted in one Rustia VM and decision epoch.
///
/// Clones share the same root lease. Values preserve table/function/userdata
/// identity, binary strings, buffers, vectors, rich keys, and cyclic/shared
/// graphs without exposing the VM's internal collector lifetime.
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

    /// Applies Luau truthiness without invoking guest code.
    pub fn truthy<C>(&self, rustia: &Rustia<C>) -> Result<bool>
    where
        C: HostSchema,
    {
        self.raw(rustia.vm(), "value handle").map(RawValue::truthy)
    }

    /// Returns the boolean payload, or `None` for another value kind.
    pub fn as_boolean<C>(&self, rustia: &Rustia<C>) -> Result<Option<bool>>
    where
        C: HostSchema,
    {
        self.raw(rustia.vm(), "value handle").map(RawValue::as_bool)
    }

    /// Returns the number payload, or `None` for another value kind.
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

    /// Returns the canonical three-component vector payload, or `None` for another kind.
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

    /// Deep-copies this value into the small acyclic owned snapshot model.
    ///
    /// Binary strings that are not UTF-8, buffers, vectors, userdata,
    /// functions, rich keys, and cyclic graphs report an explicit conversion
    /// error instead of losing data or identity.
    pub fn to_owned<C>(&self, rustia: &mut Rustia<C>) -> Result<OwnedValue>
    where
        C: HostSchema,
    {
        let value = self.raw(rustia.vm(), "value handle")?;
        snapshot_value(rustia.vm_mut(), value)
    }

    /// Deserializes this rooted value directly without first building an
    /// [`OwnedValue`] snapshot.
    ///
    /// The returned type must own all of its data; VM strings and collector
    /// values cannot escape the mutable VM borrow.
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

    /// Returns a rooted table view when this value is a table.
    pub fn as_table<C>(&self, rustia: &Rustia<C>) -> Result<Option<Table>>
    where
        C: HostSchema,
    {
        Ok((self.kind(rustia)? == ValueType::Table).then(|| Table::new(self.clone())))
    }

    /// Returns a rooted function view when this value is a function.
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
        ensure_current_epoch(label, self.decision_epoch, vm.decision_epoch())?;
        ensure_same_vm(label, &self.value, vm)?;
        self.value.value(vm).map_err(Error::from)
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

/// A rooted live table handle.
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

    /// Deserializes this rooted table directly without an owned graph snapshot.
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

    /// Performs a raw lookup with a live key, bypassing `__index`.
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

    /// Converts an owned key and performs a raw lookup, bypassing `__index`.
    pub fn get<K, C>(&self, rustia: &mut Rustia<C>, key: K) -> Result<Value>
    where
        K: IntoRustia,
        C: HostSchema,
    {
        let key = key.into_rustia()?;
        let key = load_owned_value(rustia.vm_mut(), &key)?;
        let table = self.value.raw(rustia.vm(), "table handle")?;
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

    /// Converts an owned key and writes a live value, bypassing `__newindex`.
    ///
    /// This is the identity-preserving bridge for installing a constructed or
    /// returned live object under an ordinary string/number/boolean key.
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

    /// Converts owned key/value inputs and performs a raw write, bypassing `__newindex`.
    pub fn set<K, V, C>(&self, rustia: &mut Rustia<C>, key: K, value: V) -> Result<()>
    where
        K: IntoRustia,
        V: IntoRustia,
        C: HostSchema,
    {
        let key = key.into_rustia()?;
        let value = value.into_rustia()?;
        let key = load_owned_value(rustia.vm_mut(), &key)?;
        let value = load_owned_value(rustia.vm_mut(), &value)?;
        let table = self.value.raw(rustia.vm(), "table handle")?;
        rustia
            .vm_mut()
            .raw_table_set_value(table, key, value)
            .map_err(Error::from)
    }

    /// Snapshots the current insertion-ordered entries into separately rooted handles.
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

pub(crate) fn ensure_current_epoch(
    label: &str,
    handle_epoch: u64,
    current_epoch: u64,
) -> Result<()> {
    if handle_epoch == current_epoch {
        return Ok(());
    }
    Err(Error::embed(format!(
        "stale {label} from decision epoch {handle_epoch}; current decision epoch is {current_epoch}"
    )))
}

pub(crate) fn ensure_same_vm(label: &str, value: &RetainedValue, vm: &Vm) -> Result<()> {
    if value.belongs_to(vm) {
        return Ok(());
    }
    Err(Error::embed(format!(
        "{label} belongs to a different Rustia VM"
    )))
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
