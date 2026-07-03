use crate::{Error, Result};
use rustia_vm::{Error as VmError, ErrorKind as VmErrorKind, Value as RawValue, ValueKind, Vm};
use std::collections::BTreeMap;

const SNAPSHOT_DEPTH_LIMIT: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum OwnedValue {
    Nil,
    Boolean(bool),
    Number(f64),
    String(String),
    Table(OwnedTable),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OwnedTable {
    pub array: Vec<OwnedValue>,
    pub entries: Vec<(OwnedKey, OwnedValue)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OwnedKey {
    Boolean(bool),
    Number(f64),
    String(String),
}

impl OwnedTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn array(values: impl IntoIterator<Item = OwnedValue>) -> Self {
        Self {
            array: values.into_iter().collect(),
            entries: Vec::new(),
        }
    }

    pub fn entries(entries: impl IntoIterator<Item = (OwnedKey, OwnedValue)>) -> Self {
        Self {
            array: Vec::new(),
            entries: entries.into_iter().collect(),
        }
    }

    pub fn get_string(&self, key: &str) -> Option<&OwnedValue> {
        self.entries.iter().find_map(|(entry_key, value)| {
            matches!(entry_key, OwnedKey::String(found) if found == key).then_some(value)
        })
    }
}

impl OwnedValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Nil => "nil",
            Self::Boolean(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Table(_) => "table",
        }
    }

    pub fn table(&self) -> Option<&OwnedTable> {
        match self {
            Self::Table(table) => Some(table),
            _ => None,
        }
    }
}

pub trait IntoRustia {
    fn into_rustia(self) -> OwnedValue;
}

pub trait FromRustia: Sized {
    fn from_rustia(value: OwnedValue) -> Result<Self>;
}

pub trait IntoRustiaMulti {
    fn into_rustia_multi(self) -> Vec<OwnedValue>;
}

pub trait FromRustiaMulti: Sized {
    fn from_rustia_multi(values: Vec<OwnedValue>) -> Result<Self>;
}

impl IntoRustia for OwnedValue {
    fn into_rustia(self) -> OwnedValue {
        self
    }
}

impl FromRustia for OwnedValue {
    fn from_rustia(value: OwnedValue) -> Result<Self> {
        Ok(value)
    }
}

impl IntoRustia for OwnedTable {
    fn into_rustia(self) -> OwnedValue {
        OwnedValue::Table(self)
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
    fn into_rustia(self) -> OwnedValue {
        OwnedValue::Nil
    }
}

impl FromRustia for () {
    fn from_rustia(_value: OwnedValue) -> Result<Self> {
        Ok(())
    }
}

impl IntoRustia for bool {
    fn into_rustia(self) -> OwnedValue {
        OwnedValue::Boolean(self)
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
    fn into_rustia(self) -> OwnedValue {
        OwnedValue::Number(self)
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
    fn into_rustia(self) -> OwnedValue {
        OwnedValue::Number(f64::from(self))
    }
}

impl FromRustia for f32 {
    fn from_rustia(value: OwnedValue) -> Result<Self> {
        f64::from_rustia(value).map(|value| value as f32)
    }
}

macro_rules! impl_integer_value {
    ($($ty:ty),* $(,)?) => {
        $(
            impl IntoRustia for $ty {
                fn into_rustia(self) -> OwnedValue {
                    OwnedValue::Number(self as f64)
                }
            }

            impl FromRustia for $ty {
                fn from_rustia(value: OwnedValue) -> Result<Self> {
                    let number = f64::from_rustia(value)?;
                    if !number.is_finite() || number.fract() != 0.0 {
                        return Err(Error::Embed(format!(
                            "expected integer-compatible number, got {number}"
                        )));
                    }
                    if number < <$ty>::MIN as f64 || number > <$ty>::MAX as f64 {
                        return Err(Error::Embed(format!(
                            "number {number} is outside {} range",
                            stringify!($ty)
                        )));
                    }
                    Ok(number as $ty)
                }
            }
        )*
    };
}

impl_integer_value!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl IntoRustia for String {
    fn into_rustia(self) -> OwnedValue {
        OwnedValue::String(self)
    }
}

impl IntoRustia for &str {
    fn into_rustia(self) -> OwnedValue {
        OwnedValue::String(self.to_owned())
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
    fn into_rustia(self) -> OwnedValue {
        self.map_or(OwnedValue::Nil, IntoRustia::into_rustia)
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
    fn into_rustia(self) -> OwnedValue {
        OwnedValue::Table(OwnedTable::array(
            self.into_iter().map(IntoRustia::into_rustia),
        ))
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
            OwnedValue::Table(_) => Err(Error::Embed(
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
    fn into_rustia(self) -> OwnedValue {
        OwnedValue::Table(OwnedTable::entries(
            self.into_iter()
                .map(|(key, value)| (OwnedKey::String(key), value.into_rustia())),
        ))
    }
}

impl<T> FromRustia for BTreeMap<String, T>
where
    T: FromRustia,
{
    fn from_rustia(value: OwnedValue) -> Result<Self> {
        let table = OwnedTable::from_rustia(value)?;
        if !table.array.is_empty() {
            return Err(Error::Embed(
                "expected string-keyed table, got table with array entries".to_owned(),
            ));
        }
        table
            .entries
            .into_iter()
            .map(|(key, value)| match key {
                OwnedKey::String(key) => T::from_rustia(value).map(|value| (key, value)),
                _ => Err(Error::Embed(
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
                fn into_rustia_multi(self) -> Vec<OwnedValue> {
                    vec![self.into_rustia()]
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
    fn into_rustia_multi(self) -> Vec<OwnedValue> {
        vec![self.into_rustia()]
    }
}

impl<T> IntoRustiaMulti for Option<T>
where
    T: IntoRustia,
{
    fn into_rustia_multi(self) -> Vec<OwnedValue> {
        vec![self.into_rustia()]
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
    fn into_rustia_multi(self) -> Vec<OwnedValue> {
        vec![self.into_rustia()]
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
    fn into_rustia_multi(self) -> Vec<OwnedValue> {
        vec![self.into_rustia()]
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
    fn into_rustia_multi(self) -> Vec<OwnedValue> {
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
            fn into_rustia_multi(self) -> Vec<OwnedValue> {
                let ($($name,)+) = self;
                vec![$($name.into_rustia(),)+]
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
        return Err(Error::Embed(
            "value snapshot exceeded recursive table depth limit".to_owned(),
        ));
    }

    match value.kind() {
        ValueKind::Nil => Ok(OwnedValue::Nil),
        ValueKind::Boolean => Ok(OwnedValue::Boolean(value.as_bool().unwrap_or(false))),
        ValueKind::Number => Ok(OwnedValue::Number(value.as_number().unwrap_or(f64::NAN))),
        ValueKind::String => vm
            .string_value(value)
            .map_err(Error::Vm)?
            .map(OwnedValue::String)
            .ok_or_else(|| Error::Embed("string value is missing".to_owned())),
        ValueKind::Table => snapshot_table(vm, value, depth + 1),
        ValueKind::Function => Err(Error::Embed(
            "function values must be retained as Function handles".to_owned(),
        )),
        ValueKind::Userdata | ValueKind::Buffer | ValueKind::Vector => Err(Error::Embed(format!(
            "{} values cannot be represented as OwnedValue",
            raw_kind_name(value.kind())
        ))),
        _ => Err(Error::Embed(
            "unknown VM value kind cannot be represented as OwnedValue".to_owned(),
        )),
    }
}

fn snapshot_table(vm: &mut Vm, value: RawValue, depth: usize) -> Result<OwnedValue> {
    let mut ordered = Vec::new();
    let mut all_array_keys = true;

    for (key, value) in vm.raw_table_entries_value(value).map_err(Error::Vm)? {
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
            .map_err(Error::Vm)?
            .map(OwnedKey::String)
            .ok_or_else(|| Error::Embed("string key is missing".to_owned())),
        _ => Err(Error::Embed(format!(
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
                    .map_err(Error::Vm)?;
            }
            for (key, value) in &table.entries {
                let key = load_owned_key(vm, key);
                let value = load_owned_value(vm, value)?;
                vm.raw_table_set_value(raw_table, key, value)
                    .map_err(Error::Vm)?;
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

fn type_error<T>(expected: &'static str, value: &OwnedValue) -> Result<T> {
    Err(Error::Embed(format!(
        "expected {expected}, got {}",
        value.type_name()
    )))
}

pub(crate) fn embed_to_vm_error(error: Error) -> VmError {
    match error {
        Error::Vm(error) => error,
        error => VmError::new(VmErrorKind::RuntimeError {
            message: error.to_string(),
        }),
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
