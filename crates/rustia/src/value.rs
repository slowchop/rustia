use crate::{Error, Result};
use rustia_vm::{Error as VmError, ErrorKind as VmErrorKind, Value as RawValue, ValueKind, Vm};
use std::any::type_name;
use std::collections::BTreeMap;

const SNAPSHOT_DEPTH_LIMIT: usize = 64;

/// A Rust-owned copy of a script value.
///
/// Use this for ordinary data that should outlive the VM or a
/// [`reset_decision`](crate::Rustia::reset_decision). Tables are copied
/// recursively; changes to the original script table do not affect the copy.
///
/// This representation supports UTF-8 strings and acyclic tables with boolean,
/// number, or string keys. Use a live [`crate::Value`] for binary strings,
/// functions, buffers, vectors, userdata, or shared and cyclic table identity.
#[derive(Clone, Debug, PartialEq)]
pub enum OwnedValue {
    Nil,
    Boolean(bool),
    Number(f64),
    String(String),
    Table(OwnedTable),
}

/// A Rust-owned table, split into sequence values and keyed entries.
///
/// The `array` field uses zero-based Rust indices, corresponding to Luau
/// keys `1`, `2`, and so on. When a script table is copied, a contiguous sequence
/// becomes `array`; a sparse or mixed-key table becomes `entries`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OwnedTable {
    /// Sequence values, starting at Luau index `1`.
    pub array: Vec<OwnedValue>,
    /// Keyed values in their snapshot order.
    pub entries: Vec<(OwnedKey, OwnedValue)>,
}

/// A key supported by an [`OwnedTable`].
#[derive(Clone, Debug, PartialEq)]
pub enum OwnedKey {
    Boolean(bool),
    Number(f64),
    String(String),
}

impl OwnedTable {
    /// Creates an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a sequence from values that are already converted.
    pub fn array(values: impl IntoIterator<Item = OwnedValue>) -> Self {
        Self {
            array: values.into_iter().collect(),
            entries: Vec::new(),
        }
    }

    /// Creates a table from keys and values that are already converted.
    pub fn entries(entries: impl IntoIterator<Item = (OwnedKey, OwnedValue)>) -> Self {
        Self {
            array: Vec::new(),
            entries: entries.into_iter().collect(),
        }
    }

    /// Looks up a string key without converting or cloning its value.
    pub fn get_string(&self, key: &str) -> Option<&OwnedValue> {
        self.entries.iter().find_map(|(entry_key, value)| {
            matches!(entry_key, OwnedKey::String(found) if found == key).then_some(value)
        })
    }

    /// Returns a borrowed table for a string key, or an error if the key holds a non-table value.
    pub fn get_string_table(&self, key: &str) -> Result<Option<&OwnedTable>> {
        self.get_string(key).map(expect_table_ref).transpose()
    }

    /// Returns an owned-array slot by zero-based Rust index. Luau index 1 is slot 0.
    pub fn get_array(&self, index: usize) -> Option<&OwnedValue> {
        self.array.get(index)
    }

    /// Returns a borrowed table for an array slot, or an error if the slot holds a non-table value.
    pub fn get_array_table(&self, index: usize) -> Result<Option<&OwnedTable>> {
        self.get_array(index).map(expect_table_ref).transpose()
    }
}

impl OwnedValue {
    /// Returns the corresponding Luau type name.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Nil => "nil",
            Self::Boolean(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Table(_) => "table",
        }
    }

    /// Borrows the table contents, or returns `None` for another value kind.
    pub fn table(&self) -> Option<&OwnedTable> {
        match self {
            Self::Table(table) => Some(table),
            _ => None,
        }
    }
}

/// Converts a Rust value to an owned script value.
///
/// Implementations include numbers, strings, `Option<T>`, `Vec<T>`, and
/// `BTreeMap<String, T>`. `None` becomes `nil`; vectors become one-based Luau
/// sequences. Integer conversions reject values that cannot be represented
/// exactly as a Luau number.
pub trait IntoRustia {
    /// Converts this value, returning an error if it cannot be represented.
    fn into_rustia(self) -> Result<OwnedValue>;
}

/// Converts an owned script value to a Rust value.
///
/// Conversions check the requested type rather than applying Luau coercions.
/// `Option<T>` accepts `nil` as `None`; integer types require a finite, whole
/// number in range. `Vec<T>` requires a sequence without keyed entries, while
/// `BTreeMap<String, T>` requires string keys without sequence entries.
pub trait FromRustia: Sized {
    /// Converts a value, returning an error for an incompatible type or range.
    fn from_rustia(value: OwnedValue) -> Result<Self>;
}

impl OwnedTable {
    /// Converts Rust values into a one-based Luau sequence.
    pub fn array_values<T>(values: impl IntoIterator<Item = T>) -> Result<Self>
    where
        T: IntoRustia,
    {
        Ok(Self {
            array: values
                .into_iter()
                .map(IntoRustia::into_rustia)
                .collect::<Result<Vec<_>>>()?,
            entries: Vec::new(),
        })
    }

    /// Converts Rust values into a table with string keys.
    pub fn string_entries<K, V>(entries: impl IntoIterator<Item = (K, V)>) -> Result<Self>
    where
        K: Into<String>,
        V: IntoRustia,
    {
        Ok(Self {
            array: Vec::new(),
            entries: entries
                .into_iter()
                .map(|(key, value)| {
                    value
                        .into_rustia()
                        .map(|value| (OwnedKey::String(key.into()), value))
                })
                .collect::<Result<Vec<_>>>()?,
        })
    }

    /// Reads and converts a string-keyed value. Missing keys return `None`.
    pub fn get_string_as<T>(&self, key: &str) -> Result<Option<T>>
    where
        T: FromRustia,
    {
        self.get_string(key)
            .cloned()
            .map(T::from_rustia)
            .transpose()
    }

    /// Converts an owned-array slot by zero-based Rust index. Luau index 1 is slot 0.
    pub fn get_array_as<T>(&self, index: usize) -> Result<Option<T>>
    where
        T: FromRustia,
    {
        self.get_array(index)
            .cloned()
            .map(T::from_rustia)
            .transpose()
    }
}

/// Converts function arguments or results to a list of script values.
///
/// A single value becomes one argument, `()` becomes no arguments, and tuples
/// through eight elements become separate arguments. A `Vec<T>` is one Luau
/// sequence table, not a list of arguments.
pub trait IntoRustiaMulti {
    /// Converts the complete argument or result list.
    fn into_rustia_multi(self) -> Result<Vec<OwnedValue>>;

    /// Returns Rust type names used by host-function signature metadata.
    fn rustia_type_names() -> Vec<&'static str>
    where
        Self: Sized,
    {
        vec![type_name::<Self>()]
    }
}

/// Converts a script's return values to a Rust value or tuple.
///
/// A single type reads the first result; tuples through eight elements read
/// successive results. Missing results are treated as `nil`, extra results are
/// ignored, and `()` discards all results. Each selected result still has to
/// satisfy its [`FromRustia`] conversion.
///
/// Owned execution methods copy every result into [`OwnedValue`] before calling
/// this trait, including results that will be discarded. Use live-value methods
/// when any result cannot be represented by [`OwnedValue`].
pub trait FromRustiaMulti: Sized {
    /// Converts the selected return values.
    fn from_rustia_multi(values: Vec<OwnedValue>) -> Result<Self>;
}

impl IntoRustia for OwnedValue {
    fn into_rustia(self) -> Result<OwnedValue> {
        Ok(self)
    }
}

impl FromRustia for OwnedValue {
    fn from_rustia(value: OwnedValue) -> Result<Self> {
        Ok(value)
    }
}

impl IntoRustia for OwnedTable {
    fn into_rustia(self) -> Result<OwnedValue> {
        Ok(OwnedValue::Table(self))
    }
}

impl FromRustia for OwnedTable {
    fn from_rustia(value: OwnedValue) -> Result<Self> {
        match value {
            OwnedValue::Table(table) => Ok(table),
            other => type_error("table", &other),
        }
    }
}

impl IntoRustia for () {
    fn into_rustia(self) -> Result<OwnedValue> {
        Ok(OwnedValue::Nil)
    }
}

impl FromRustia for () {
    fn from_rustia(_value: OwnedValue) -> Result<Self> {
        Ok(())
    }
}

impl IntoRustia for bool {
    fn into_rustia(self) -> Result<OwnedValue> {
        Ok(OwnedValue::Boolean(self))
    }
}

impl FromRustia for bool {
    fn from_rustia(value: OwnedValue) -> Result<Self> {
        match value {
            OwnedValue::Boolean(value) => Ok(value),
            other => type_error("boolean", &other),
        }
    }
}

impl IntoRustia for f64 {
    fn into_rustia(self) -> Result<OwnedValue> {
        Ok(OwnedValue::Number(self))
    }
}

impl FromRustia for f64 {
    fn from_rustia(value: OwnedValue) -> Result<Self> {
        match value {
            OwnedValue::Number(value) => Ok(value),
            other => type_error("number", &other),
        }
    }
}

impl IntoRustia for f32 {
    fn into_rustia(self) -> Result<OwnedValue> {
        Ok(OwnedValue::Number(f64::from(self)))
    }
}

impl FromRustia for f32 {
    fn from_rustia(value: OwnedValue) -> Result<Self> {
        f64::from_rustia(value).map(|value| value as f32)
    }
}

macro_rules! impl_signed_integer_value {
    ($($ty:ty),* $(,)?) => {
        $(
            impl IntoRustia for $ty {
                fn into_rustia(self) -> Result<OwnedValue> {
                    let number = self as f64;
                    if number as i128 != self as i128 {
                        return Err(Error::embed(format!(
                            "{} value {self} cannot be represented exactly as a Luau number",
                            stringify!($ty)
                        )));
                    }
                    Ok(OwnedValue::Number(number))
                }
            }

            impl FromRustia for $ty {
                fn from_rustia(value: OwnedValue) -> Result<Self> {
                    let number = f64::from_rustia(value)?;
                    if !number.is_finite() || number.fract() != 0.0 {
                        return Err(Error::embed(format!(
                            "expected integer-compatible number, got {number}"
                        )));
                    }
                    let integer = number as i128;
                    if integer < <$ty>::MIN as i128 || integer > <$ty>::MAX as i128 {
                        return Err(Error::embed(format!(
                            "number {number} is outside {} range",
                            stringify!($ty)
                        )));
                    }
                    Ok(integer as $ty)
                }
            }
        )*
    };
}

macro_rules! impl_unsigned_integer_value {
    ($($ty:ty),* $(,)?) => {
        $(
            impl IntoRustia for $ty {
                fn into_rustia(self) -> Result<OwnedValue> {
                    let number = self as f64;
                    if number as u128 != self as u128 {
                        return Err(Error::embed(format!(
                            "{} value {self} cannot be represented exactly as a Luau number",
                            stringify!($ty)
                        )));
                    }
                    Ok(OwnedValue::Number(number))
                }
            }

            impl FromRustia for $ty {
                fn from_rustia(value: OwnedValue) -> Result<Self> {
                    let number = f64::from_rustia(value)?;
                    if !number.is_finite() || number.fract() != 0.0 {
                        return Err(Error::embed(format!(
                            "expected integer-compatible number, got {number}"
                        )));
                    }
                    if number < 0.0 {
                        return Err(Error::embed(format!(
                            "number {number} is outside {} range",
                            stringify!($ty)
                        )));
                    }
                    let integer = number as u128;
                    if integer > <$ty>::MAX as u128 {
                        return Err(Error::embed(format!(
                            "number {number} is outside {} range",
                            stringify!($ty)
                        )));
                    }
                    Ok(integer as $ty)
                }
            }
        )*
    };
}

impl_signed_integer_value!(i8, i16, i32, i64, isize);
impl_unsigned_integer_value!(u8, u16, u32, u64, usize);

impl IntoRustia for String {
    fn into_rustia(self) -> Result<OwnedValue> {
        Ok(OwnedValue::String(self))
    }
}

impl IntoRustia for &str {
    fn into_rustia(self) -> Result<OwnedValue> {
        Ok(OwnedValue::String(self.to_owned()))
    }
}

impl FromRustia for String {
    fn from_rustia(value: OwnedValue) -> Result<Self> {
        match value {
            OwnedValue::String(value) => Ok(value),
            other => type_error("string", &other),
        }
    }
}

impl<T> IntoRustia for Option<T>
where
    T: IntoRustia,
{
    fn into_rustia(self) -> Result<OwnedValue> {
        self.map_or(Ok(OwnedValue::Nil), IntoRustia::into_rustia)
    }
}

impl<T> FromRustia for Option<T>
where
    T: FromRustia,
{
    fn from_rustia(value: OwnedValue) -> Result<Self> {
        match value {
            OwnedValue::Nil => Ok(None),
            value => T::from_rustia(value).map(Some),
        }
    }
}

impl<T> IntoRustia for Vec<T>
where
    T: IntoRustia,
{
    fn into_rustia(self) -> Result<OwnedValue> {
        OwnedTable::array_values(self).map(OwnedValue::Table)
    }
}

impl<T> FromRustia for Vec<T>
where
    T: FromRustia,
{
    fn from_rustia(value: OwnedValue) -> Result<Self> {
        match value {
            OwnedValue::Table(table) if table.entries.is_empty() => {
                table.array.into_iter().map(T::from_rustia).collect()
            }
            OwnedValue::Table(_) => Err(Error::embed(
                "expected array table, got table with keyed entries".to_owned(),
            )),
            other => type_error("array table", &other),
        }
    }
}

impl<T> IntoRustia for BTreeMap<String, T>
where
    T: IntoRustia,
{
    fn into_rustia(self) -> Result<OwnedValue> {
        OwnedTable::string_entries(self).map(OwnedValue::Table)
    }
}

impl<T> FromRustia for BTreeMap<String, T>
where
    T: FromRustia,
{
    fn from_rustia(value: OwnedValue) -> Result<Self> {
        let table = OwnedTable::from_rustia(value)?;
        if !table.array.is_empty() {
            return Err(Error::embed(
                "expected string-keyed table, got table with array entries".to_owned(),
            ));
        }
        table
            .entries
            .into_iter()
            .map(|(key, value)| match key {
                OwnedKey::String(key) => T::from_rustia(value).map(|value| (key, value)),
                _ => Err(Error::embed(
                    "expected string-keyed table, got non-string key".to_owned(),
                )),
            })
            .collect()
    }
}

macro_rules! impl_single_multi {
    ($($ty:ty),* $(,)?) => {
        $(
            impl IntoRustiaMulti for $ty {
                fn into_rustia_multi(self) -> Result<Vec<OwnedValue>> {
                    self.into_rustia().map(|value| vec![value])
                }
            }

            impl FromRustiaMulti for $ty {
                fn from_rustia_multi(mut values: Vec<OwnedValue>) -> Result<Self> {
                    let value = if values.is_empty() {
                        OwnedValue::Nil
                    } else {
                        values.remove(0)
                    };
                    Self::from_rustia(value)
                }
            }
        )*
    };
}

impl_single_multi!(
    OwnedValue, OwnedTable, bool, f64, f32, i8, i16, i32, i64, isize, u8, u16, u32, u64, usize,
    String
);

impl IntoRustiaMulti for &str {
    fn into_rustia_multi(self) -> Result<Vec<OwnedValue>> {
        self.into_rustia().map(|value| vec![value])
    }
}

impl<T> IntoRustiaMulti for Option<T>
where
    T: IntoRustia,
{
    fn into_rustia_multi(self) -> Result<Vec<OwnedValue>> {
        self.into_rustia().map(|value| vec![value])
    }
}

impl<T> FromRustiaMulti for Option<T>
where
    T: FromRustia,
{
    fn from_rustia_multi(mut values: Vec<OwnedValue>) -> Result<Self> {
        let value = if values.is_empty() {
            OwnedValue::Nil
        } else {
            values.remove(0)
        };
        Self::from_rustia(value)
    }
}

impl<T> IntoRustiaMulti for Vec<T>
where
    T: IntoRustia,
{
    fn into_rustia_multi(self) -> Result<Vec<OwnedValue>> {
        self.into_rustia().map(|value| vec![value])
    }
}

impl<T> FromRustiaMulti for Vec<T>
where
    T: FromRustia,
{
    fn from_rustia_multi(mut values: Vec<OwnedValue>) -> Result<Self> {
        let value = if values.is_empty() {
            OwnedValue::Nil
        } else {
            values.remove(0)
        };
        Self::from_rustia(value)
    }
}

impl<T> IntoRustiaMulti for BTreeMap<String, T>
where
    T: IntoRustia,
{
    fn into_rustia_multi(self) -> Result<Vec<OwnedValue>> {
        self.into_rustia().map(|value| vec![value])
    }
}

impl<T> FromRustiaMulti for BTreeMap<String, T>
where
    T: FromRustia,
{
    fn from_rustia_multi(mut values: Vec<OwnedValue>) -> Result<Self> {
        let value = if values.is_empty() {
            OwnedValue::Nil
        } else {
            values.remove(0)
        };
        Self::from_rustia(value)
    }
}

impl IntoRustiaMulti for () {
    fn into_rustia_multi(self) -> Result<Vec<OwnedValue>> {
        Ok(Vec::new())
    }

    fn rustia_type_names() -> Vec<&'static str> {
        Vec::new()
    }
}

impl FromRustiaMulti for () {
    fn from_rustia_multi(_values: Vec<OwnedValue>) -> Result<Self> {
        Ok(())
    }
}

macro_rules! impl_tuple_multi {
    ($($name:ident),+ $(,)?) => {
        impl<$($name),+> IntoRustiaMulti for ($($name,)+)
        where
            $($name: IntoRustia,)+
        {
            #[allow(non_snake_case)]
            fn into_rustia_multi(self) -> Result<Vec<OwnedValue>> {
                let ($($name,)+) = self;
                Ok(vec![$($name.into_rustia()?,)+])
            }

            fn rustia_type_names() -> Vec<&'static str> {
                vec![$(type_name::<$name>()),+]
            }
        }

        impl<$($name),+> FromRustiaMulti for ($($name,)+)
        where
            $($name: FromRustia,)+
        {
            #[allow(non_snake_case)]
            fn from_rustia_multi(values: Vec<OwnedValue>) -> Result<Self> {
                let mut values = values.into_iter();
                Ok(($({
                    let value = values.next().unwrap_or(OwnedValue::Nil);
                    $name::from_rustia(value)?
                },)+))
            }
        }
    };
}

impl_tuple_multi!(A);
impl_tuple_multi!(A, B);
impl_tuple_multi!(A, B, C);
impl_tuple_multi!(A, B, C, D);
impl_tuple_multi!(A, B, C, D, E);
impl_tuple_multi!(A, B, C, D, E, F);
impl_tuple_multi!(A, B, C, D, E, F, G);
impl_tuple_multi!(A, B, C, D, E, F, G, H);

pub(crate) fn snapshot_value(vm: &mut Vm, value: RawValue) -> Result<OwnedValue> {
    snapshot_value_inner(vm, value, 0)
}

fn snapshot_value_inner(vm: &mut Vm, value: RawValue, depth: usize) -> Result<OwnedValue> {
    if depth > SNAPSHOT_DEPTH_LIMIT {
        return Err(Error::embed(
            "value snapshot exceeded recursive table depth limit".to_owned(),
        ));
    }

    match value.kind() {
        ValueKind::Nil => Ok(OwnedValue::Nil),
        ValueKind::Boolean => Ok(OwnedValue::Boolean(value.as_bool().unwrap_or(false))),
        ValueKind::Number => Ok(OwnedValue::Number(value.as_number().unwrap_or(f64::NAN))),
        ValueKind::String => vm
            .string_value(value)
            .map_err(Error::from)?
            .map(OwnedValue::String)
            .ok_or_else(|| Error::embed("string value is missing".to_owned())),
        ValueKind::Table => snapshot_table(vm, value, depth + 1),
        ValueKind::Function => Err(Error::embed(
            "function values must be retained as Function handles".to_owned(),
        )),
        ValueKind::Userdata | ValueKind::Buffer | ValueKind::Vector => Err(Error::embed(format!(
            "{} values cannot be represented as OwnedValue",
            raw_kind_name(value.kind())
        ))),
        _ => Err(Error::embed(
            "unknown VM value kind cannot be represented as OwnedValue".to_owned(),
        )),
    }
}

fn snapshot_table(vm: &mut Vm, value: RawValue, depth: usize) -> Result<OwnedValue> {
    let mut ordered = Vec::new();
    let mut all_array_keys = true;

    for (key, value) in vm.raw_table_entries_value(value).map_err(Error::from)? {
        let key = snapshot_key(vm, key)?;
        let array_index = match key {
            OwnedKey::Number(number) => number_to_array_index(number),
            _ => None,
        };
        if array_index.is_none() {
            all_array_keys = false;
        }
        ordered.push((
            key,
            snapshot_value_inner(vm, value, depth + 1)?,
            array_index,
        ));
    }

    if all_array_keys {
        let mut array_entries = ordered
            .iter()
            .filter_map(|(_, value, index)| index.map(|index| (index, value.clone())))
            .collect::<Vec<_>>();
        array_entries.sort_by_key(|(index, _)| *index);
        let contiguous = array_entries
            .iter()
            .enumerate()
            .all(|(position, (index, _))| *index == position + 1);
        if contiguous {
            let array = array_entries
                .into_iter()
                .map(|(_, value)| value)
                .collect::<Vec<_>>();
            return Ok(OwnedValue::Table(OwnedTable {
                array,
                entries: Vec::new(),
            }));
        }
    }

    Ok(OwnedValue::Table(OwnedTable {
        array: Vec::new(),
        entries: ordered
            .into_iter()
            .map(|(key, value, _)| (key, value))
            .collect(),
    }))
}

fn snapshot_key(vm: &mut Vm, key: RawValue) -> Result<OwnedKey> {
    match key.kind() {
        ValueKind::Boolean => Ok(OwnedKey::Boolean(key.as_bool().unwrap_or(false))),
        ValueKind::Number => Ok(OwnedKey::Number(key.as_number().unwrap_or(f64::NAN))),
        ValueKind::String => vm
            .string_value(key)
            .map_err(Error::from)?
            .map(OwnedKey::String)
            .ok_or_else(|| Error::embed("string key is missing".to_owned())),
        _ => Err(Error::embed(format!(
            "{} table keys cannot be represented as OwnedKey",
            raw_kind_name(key.kind())
        ))),
    }
}

pub(crate) fn load_owned_value(vm: &mut Vm, value: &OwnedValue) -> Result<RawValue> {
    match value {
        OwnedValue::Nil => Ok(RawValue::nil()),
        OwnedValue::Boolean(value) => Ok(RawValue::boolean(*value)),
        OwnedValue::Number(value) => Ok(RawValue::number(*value)),
        OwnedValue::String(value) => Ok(vm.string(value)),
        OwnedValue::Table(table) => {
            let raw_table = vm.empty_table();
            for (index, value) in table.array.iter().enumerate() {
                let key = RawValue::number((index + 1) as f64);
                let value = load_owned_value(vm, value)?;
                vm.raw_table_set_value(raw_table, key, value)
                    .map_err(Error::from)?;
            }
            for (key, value) in &table.entries {
                let key = load_owned_key(vm, key);
                let value = load_owned_value(vm, value)?;
                vm.raw_table_set_value(raw_table, key, value)
                    .map_err(Error::from)?;
            }
            Ok(raw_table)
        }
    }
}

fn load_owned_key(vm: &mut Vm, key: &OwnedKey) -> RawValue {
    match key {
        OwnedKey::Boolean(value) => RawValue::boolean(*value),
        OwnedKey::Number(value) => RawValue::number(*value),
        OwnedKey::String(value) => vm.string(value),
    }
}

fn number_to_array_index(number: f64) -> Option<usize> {
    if !number.is_finite() || number.fract() != 0.0 || number < 1.0 {
        return None;
    }
    if number > usize::MAX as f64 {
        return None;
    }
    Some(number as usize)
}

fn expect_table_ref(value: &OwnedValue) -> Result<&OwnedTable> {
    match value {
        OwnedValue::Table(table) => Ok(table),
        other => type_error("table", other),
    }
}

fn type_error<T>(expected: &'static str, value: &OwnedValue) -> Result<T> {
    Err(Error::embed(format!(
        "expected {expected}, got {}",
        value.type_name()
    )))
}

pub(crate) fn embed_to_vm_error(error: Error) -> VmError {
    match error {
        Error {
            kind: crate::ErrorKind::Vm(error),
            ..
        } => *error,
        error => {
            let mut vm_error = VmError::new(VmErrorKind::RuntimeError {
                message: error.to_string(),
            });
            vm_error.diagnostic = Some(std::sync::Arc::new(crate::Diagnostic {
                host_cause: Some(std::sync::Arc::new(error)),
                ..crate::Diagnostic::default()
            }));
            vm_error
        }
    }
}

fn raw_kind_name(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Nil => "nil",
        ValueKind::Boolean => "boolean",
        ValueKind::Number => "number",
        ValueKind::String => "string",
        ValueKind::Table => "table",
        ValueKind::Function => "function",
        ValueKind::Userdata => "userdata",
        ValueKind::Buffer => "buffer",
        ValueKind::Vector => "vector",
        _ => "unknown",
    }
}
