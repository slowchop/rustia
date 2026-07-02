use crate::{Error, OwnedKey, OwnedTable, OwnedValue, Result};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{
    self, Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant,
    SerializeTuple, SerializeTupleStruct, SerializeTupleVariant,
};
use std::fmt;

#[derive(Debug)]
pub struct SerdeValueError(String);

impl fmt::Display for SerdeValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SerdeValueError {}

impl de::Error for SerdeValueError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self(message.to_string())
    }
}

impl ser::Error for SerdeValueError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self(message.to_string())
    }
}

type SerdeResult<T> = std::result::Result<T, SerdeValueError>;

pub fn from_owned<'de, T>(value: &'de OwnedValue) -> Result<T>
where
    T: Deserialize<'de>,
{
    T::deserialize(OwnedDeserializer { value }).map_err(|error| Error::Embed(error.to_string()))
}

pub fn to_owned<T>(value: &T) -> Result<OwnedValue>
where
    T: Serialize + ?Sized,
{
    value
        .serialize(OwnedSerializer)
        .map_err(|error| Error::Embed(error.to_string()))
}

impl OwnedValue {
    pub fn deserialize<'de, T>(&'de self) -> Result<T>
    where
        T: Deserialize<'de>,
    {
        from_owned(self)
    }

    pub fn from_serialize<T>(value: &T) -> Result<Self>
    where
        T: Serialize + ?Sized,
    {
        to_owned(value)
    }
}

struct OwnedDeserializer<'de> {
    value: &'de OwnedValue,
}

impl<'de> Deserializer<'de> for OwnedDeserializer<'de> {
    type Error = SerdeValueError;

    fn deserialize_any<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Nil => visitor.visit_unit(),
            OwnedValue::Boolean(value) => visitor.visit_bool(*value),
            OwnedValue::Number(value) => visitor.visit_f64(*value),
            OwnedValue::String(value) => visitor.visit_borrowed_str(value),
            OwnedValue::Table(table) => {
                if table.entries.is_empty() {
                    visitor.visit_seq(TableSeqAccess {
                        values: table.array.iter(),
                    })
                } else {
                    visitor.visit_map(TableMapAccess::new(table))
                }
            }
        }
    }

    fn deserialize_bool<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Boolean(value) => visitor.visit_bool(*value),
            value => Err(SerdeValueError(format!(
                "expected boolean, got {}",
                value.type_name()
            ))),
        }
    }

    fn deserialize_i64<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Number(value) => visitor.visit_i64(number_to_i64(*value)?),
            value => Err(SerdeValueError(format!(
                "expected number, got {}",
                value.type_name()
            ))),
        }
    }

    fn deserialize_i8<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i8(integer_in_range(number_to_i64_value(self.value)?)?)
    }

    fn deserialize_i16<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i16(integer_in_range(number_to_i64_value(self.value)?)?)
    }

    fn deserialize_i32<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i32(integer_in_range(number_to_i64_value(self.value)?)?)
    }

    fn deserialize_i128<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i128(i128::from(number_to_i64_value(self.value)?))
    }

    fn deserialize_u64<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Number(value) => visitor.visit_u64(number_to_u64(*value)?),
            value => Err(SerdeValueError(format!(
                "expected number, got {}",
                value.type_name()
            ))),
        }
    }

    fn deserialize_u8<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u8(integer_in_range(number_to_u64_value(self.value)?)?)
    }

    fn deserialize_u16<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u16(integer_in_range(number_to_u64_value(self.value)?)?)
    }

    fn deserialize_u32<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u32(integer_in_range(number_to_u64_value(self.value)?)?)
    }

    fn deserialize_u128<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u128(u128::from(number_to_u64_value(self.value)?))
    }

    fn deserialize_f64<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Number(value) => visitor.visit_f64(*value),
            value => Err(SerdeValueError(format!(
                "expected number, got {}",
                value.type_name()
            ))),
        }
    }

    fn deserialize_f32<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Number(value) => visitor.visit_f32(*value as f32),
            value => Err(SerdeValueError(format!(
                "expected number, got {}",
                value.type_name()
            ))),
        }
    }

    fn deserialize_option<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Nil => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_str<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::String(value) => visitor.visit_borrowed_str(value),
            value => Err(SerdeValueError(format!(
                "expected string, got {}",
                value.type_name()
            ))),
        }
    }

    fn deserialize_string<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_seq<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Table(table) => visitor.visit_seq(TableSeqAccess {
                values: table.array.iter(),
            }),
            value => Err(SerdeValueError(format!(
                "expected table sequence, got {}",
                value.type_name()
            ))),
        }
    }

    fn deserialize_map<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Table(table) => visitor.visit_map(TableMapAccess::new(table)),
            value => Err(SerdeValueError(format!(
                "expected table map, got {}",
                value.type_name()
            ))),
        }
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_map(visitor)
    }

    fn deserialize_newtype_struct<V>(self, _name: &'static str, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    serde::forward_to_deserialize_any! {
        char bytes byte_buf unit unit_struct tuple tuple_struct enum identifier ignored_any
    }
}

struct TableSeqAccess<'de> {
    values: std::slice::Iter<'de, OwnedValue>,
}

impl<'de> SeqAccess<'de> for TableSeqAccess<'de> {
    type Error = SerdeValueError;

    fn next_element_seed<T>(&mut self, seed: T) -> SerdeResult<Option<T::Value>>
    where
        T: de::DeserializeSeed<'de>,
    {
        match self.values.next() {
            Some(value) => seed.deserialize(OwnedDeserializer { value }).map(Some),
            None => Ok(None),
        }
    }
}

struct TableMapAccess<'de> {
    table: &'de OwnedTable,
    array_index: usize,
    entry_index: usize,
    pending: Option<&'de OwnedValue>,
}

impl<'de> TableMapAccess<'de> {
    fn new(table: &'de OwnedTable) -> Self {
        Self {
            table,
            array_index: 0,
            entry_index: 0,
            pending: None,
        }
    }
}

impl<'de> MapAccess<'de> for TableMapAccess<'de> {
    type Error = SerdeValueError;

    fn next_key_seed<K>(&mut self, seed: K) -> SerdeResult<Option<K::Value>>
    where
        K: de::DeserializeSeed<'de>,
    {
        if self.pending.is_some() {
            return Err(SerdeValueError(
                "map value requested before previous value was consumed".to_owned(),
            ));
        }

        if let Some(value) = self.table.array.get(self.array_index) {
            let key = MapKeyDeserializer::Number((self.array_index + 1) as f64);
            self.array_index += 1;
            self.pending = Some(value);
            return seed.deserialize(key).map(Some);
        }

        if let Some((key, value)) = self.table.entries.get(self.entry_index) {
            self.entry_index += 1;
            self.pending = Some(value);
            return seed.deserialize(MapKeyDeserializer::Owned(key)).map(Some);
        }

        Ok(None)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> SerdeResult<V::Value>
    where
        V: de::DeserializeSeed<'de>,
    {
        let value = self.pending.take().ok_or_else(|| {
            SerdeValueError("map value requested before a key was produced".to_owned())
        })?;
        seed.deserialize(OwnedDeserializer { value })
    }
}

enum MapKeyDeserializer<'de> {
    Owned(&'de OwnedKey),
    Number(f64),
}

impl<'de> Deserializer<'de> for MapKeyDeserializer<'de> {
    type Error = SerdeValueError;

    fn deserialize_any<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match self {
            Self::Owned(OwnedKey::Boolean(value)) => visitor.visit_bool(*value),
            Self::Owned(OwnedKey::Number(value)) => visitor.visit_f64(*value),
            Self::Number(value) => visitor.visit_f64(value),
            Self::Owned(OwnedKey::String(value)) => visitor.visit_borrowed_str(value),
        }
    }

    fn deserialize_identifier<V>(self, visitor: V) -> SerdeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match self {
            Self::Owned(OwnedKey::String(value)) => visitor.visit_borrowed_str(value),
            _ => self.deserialize_any(visitor),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple tuple_struct
        map struct enum ignored_any
    }
}

struct OwnedSerializer;

impl ser::Serializer for OwnedSerializer {
    type Ok = OwnedValue;
    type Error = SerdeValueError;
    type SerializeSeq = ArraySerializer;
    type SerializeTuple = ArraySerializer;
    type SerializeTupleStruct = ArraySerializer;
    type SerializeTupleVariant = TupleVariantSerializer;
    type SerializeMap = MapSerializer;
    type SerializeStruct = StructSerializer;
    type SerializeStructVariant = StructVariantSerializer;

    fn serialize_bool(self, value: bool) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Boolean(value))
    }

    fn serialize_i8(self, value: i8) -> SerdeResult<Self::Ok> {
        self.serialize_i64(i64::from(value))
    }

    fn serialize_i16(self, value: i16) -> SerdeResult<Self::Ok> {
        self.serialize_i64(i64::from(value))
    }

    fn serialize_i32(self, value: i32) -> SerdeResult<Self::Ok> {
        self.serialize_i64(i64::from(value))
    }

    fn serialize_i64(self, value: i64) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Number(value as f64))
    }

    fn serialize_i128(self, value: i128) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Number(value as f64))
    }

    fn serialize_u8(self, value: u8) -> SerdeResult<Self::Ok> {
        self.serialize_u64(u64::from(value))
    }

    fn serialize_u16(self, value: u16) -> SerdeResult<Self::Ok> {
        self.serialize_u64(u64::from(value))
    }

    fn serialize_u32(self, value: u32) -> SerdeResult<Self::Ok> {
        self.serialize_u64(u64::from(value))
    }

    fn serialize_u64(self, value: u64) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Number(value as f64))
    }

    fn serialize_u128(self, value: u128) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Number(value as f64))
    }

    fn serialize_f32(self, value: f32) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Number(f64::from(value)))
    }

    fn serialize_f64(self, value: f64) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Number(value))
    }

    fn serialize_char(self, value: char) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::String(value.to_string()))
    }

    fn serialize_str(self, value: &str) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::String(value.to_owned()))
    }

    fn serialize_bytes(self, value: &[u8]) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Table(OwnedTable::array(
            value
                .iter()
                .copied()
                .map(|byte| OwnedValue::Number(f64::from(byte))),
        )))
    }

    fn serialize_none(self) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Nil)
    }

    fn serialize_some<T>(self, value: &T) -> SerdeResult<Self::Ok>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Nil)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Nil)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::String(variant.to_owned()))
    }

    fn serialize_newtype_struct<T>(self, _name: &'static str, value: &T) -> SerdeResult<Self::Ok>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> SerdeResult<Self::Ok>
    where
        T: Serialize + ?Sized,
    {
        Ok(OwnedValue::Table(OwnedTable::entries([(
            OwnedKey::String(variant.to_owned()),
            value.serialize(OwnedSerializer)?,
        )])))
    }

    fn serialize_seq(self, _len: Option<usize>) -> SerdeResult<Self::SerializeSeq> {
        Ok(ArraySerializer { values: Vec::new() })
    }

    fn serialize_tuple(self, _len: usize) -> SerdeResult<Self::SerializeTuple> {
        Ok(ArraySerializer { values: Vec::new() })
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> SerdeResult<Self::SerializeTupleStruct> {
        Ok(ArraySerializer { values: Vec::new() })
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> SerdeResult<Self::SerializeTupleVariant> {
        Ok(TupleVariantSerializer {
            name: variant.to_owned(),
            values: Vec::new(),
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> SerdeResult<Self::SerializeMap> {
        Ok(MapSerializer {
            entries: Vec::new(),
            next_key: None,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> SerdeResult<Self::SerializeStruct> {
        Ok(StructSerializer {
            entries: Vec::new(),
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> SerdeResult<Self::SerializeStructVariant> {
        Ok(StructVariantSerializer {
            name: variant.to_owned(),
            entries: Vec::new(),
        })
    }
}

struct ArraySerializer {
    values: Vec<OwnedValue>,
}

impl SerializeSeq for ArraySerializer {
    type Ok = OwnedValue;
    type Error = SerdeValueError;

    fn serialize_element<T>(&mut self, value: &T) -> SerdeResult<()>
    where
        T: Serialize + ?Sized,
    {
        self.values.push(value.serialize(OwnedSerializer)?);
        Ok(())
    }

    fn end(self) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Table(OwnedTable::array(self.values)))
    }
}

impl SerializeTuple for ArraySerializer {
    type Ok = OwnedValue;
    type Error = SerdeValueError;

    fn serialize_element<T>(&mut self, value: &T) -> SerdeResult<()>
    where
        T: Serialize + ?Sized,
    {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> SerdeResult<Self::Ok> {
        SerializeSeq::end(self)
    }
}

impl SerializeTupleStruct for ArraySerializer {
    type Ok = OwnedValue;
    type Error = SerdeValueError;

    fn serialize_field<T>(&mut self, value: &T) -> SerdeResult<()>
    where
        T: Serialize + ?Sized,
    {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> SerdeResult<Self::Ok> {
        SerializeSeq::end(self)
    }
}

struct TupleVariantSerializer {
    name: String,
    values: Vec<OwnedValue>,
}

impl SerializeTupleVariant for TupleVariantSerializer {
    type Ok = OwnedValue;
    type Error = SerdeValueError;

    fn serialize_field<T>(&mut self, value: &T) -> SerdeResult<()>
    where
        T: Serialize + ?Sized,
    {
        self.values.push(value.serialize(OwnedSerializer)?);
        Ok(())
    }

    fn end(self) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Table(OwnedTable::entries([(
            OwnedKey::String(self.name),
            OwnedValue::Table(OwnedTable::array(self.values)),
        )])))
    }
}

struct MapSerializer {
    entries: Vec<(OwnedKey, OwnedValue)>,
    next_key: Option<OwnedKey>,
}

impl SerializeMap for MapSerializer {
    type Ok = OwnedValue;
    type Error = SerdeValueError;

    fn serialize_key<T>(&mut self, key: &T) -> SerdeResult<()>
    where
        T: Serialize + ?Sized,
    {
        if self.next_key.is_some() {
            return Err(SerdeValueError(
                "serialize_key called before previous value".to_owned(),
            ));
        }
        self.next_key = Some(key.serialize(KeySerializer)?);
        Ok(())
    }

    fn serialize_value<T>(&mut self, value: &T) -> SerdeResult<()>
    where
        T: Serialize + ?Sized,
    {
        let key = self
            .next_key
            .take()
            .ok_or_else(|| SerdeValueError("serialize_value called before key".to_owned()))?;
        self.entries.push((key, value.serialize(OwnedSerializer)?));
        Ok(())
    }

    fn end(self) -> SerdeResult<Self::Ok> {
        if self.next_key.is_some() {
            return Err(SerdeValueError("map key is missing value".to_owned()));
        }
        Ok(OwnedValue::Table(OwnedTable::entries(self.entries)))
    }
}

struct StructSerializer {
    entries: Vec<(OwnedKey, OwnedValue)>,
}

impl SerializeStruct for StructSerializer {
    type Ok = OwnedValue;
    type Error = SerdeValueError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> SerdeResult<()>
    where
        T: Serialize + ?Sized,
    {
        self.entries.push((
            OwnedKey::String(key.to_owned()),
            value.serialize(OwnedSerializer)?,
        ));
        Ok(())
    }

    fn end(self) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Table(OwnedTable::entries(self.entries)))
    }
}

struct StructVariantSerializer {
    name: String,
    entries: Vec<(OwnedKey, OwnedValue)>,
}

impl SerializeStructVariant for StructVariantSerializer {
    type Ok = OwnedValue;
    type Error = SerdeValueError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> SerdeResult<()>
    where
        T: Serialize + ?Sized,
    {
        self.entries.push((
            OwnedKey::String(key.to_owned()),
            value.serialize(OwnedSerializer)?,
        ));
        Ok(())
    }

    fn end(self) -> SerdeResult<Self::Ok> {
        Ok(OwnedValue::Table(OwnedTable::entries([(
            OwnedKey::String(self.name),
            OwnedValue::Table(OwnedTable::entries(self.entries)),
        )])))
    }
}

struct KeySerializer;

impl ser::Serializer for KeySerializer {
    type Ok = OwnedKey;
    type Error = SerdeValueError;
    type SerializeSeq = ser::Impossible<OwnedKey, SerdeValueError>;
    type SerializeTuple = ser::Impossible<OwnedKey, SerdeValueError>;
    type SerializeTupleStruct = ser::Impossible<OwnedKey, SerdeValueError>;
    type SerializeTupleVariant = ser::Impossible<OwnedKey, SerdeValueError>;
    type SerializeMap = ser::Impossible<OwnedKey, SerdeValueError>;
    type SerializeStruct = ser::Impossible<OwnedKey, SerdeValueError>;
    type SerializeStructVariant = ser::Impossible<OwnedKey, SerdeValueError>;

    fn serialize_bool(self, value: bool) -> SerdeResult<Self::Ok> {
        Ok(OwnedKey::Boolean(value))
    }

    fn serialize_i8(self, value: i8) -> SerdeResult<Self::Ok> {
        self.serialize_i64(i64::from(value))
    }

    fn serialize_i16(self, value: i16) -> SerdeResult<Self::Ok> {
        self.serialize_i64(i64::from(value))
    }

    fn serialize_i32(self, value: i32) -> SerdeResult<Self::Ok> {
        self.serialize_i64(i64::from(value))
    }

    fn serialize_i64(self, value: i64) -> SerdeResult<Self::Ok> {
        Ok(OwnedKey::Number(value as f64))
    }

    fn serialize_i128(self, value: i128) -> SerdeResult<Self::Ok> {
        Ok(OwnedKey::Number(value as f64))
    }

    fn serialize_u8(self, value: u8) -> SerdeResult<Self::Ok> {
        self.serialize_u64(u64::from(value))
    }

    fn serialize_u16(self, value: u16) -> SerdeResult<Self::Ok> {
        self.serialize_u64(u64::from(value))
    }

    fn serialize_u32(self, value: u32) -> SerdeResult<Self::Ok> {
        self.serialize_u64(u64::from(value))
    }

    fn serialize_u64(self, value: u64) -> SerdeResult<Self::Ok> {
        Ok(OwnedKey::Number(value as f64))
    }

    fn serialize_u128(self, value: u128) -> SerdeResult<Self::Ok> {
        Ok(OwnedKey::Number(value as f64))
    }

    fn serialize_f32(self, value: f32) -> SerdeResult<Self::Ok> {
        Ok(OwnedKey::Number(f64::from(value)))
    }

    fn serialize_f64(self, value: f64) -> SerdeResult<Self::Ok> {
        Ok(OwnedKey::Number(value))
    }

    fn serialize_char(self, value: char) -> SerdeResult<Self::Ok> {
        Ok(OwnedKey::String(value.to_string()))
    }

    fn serialize_str(self, value: &str) -> SerdeResult<Self::Ok> {
        Ok(OwnedKey::String(value.to_owned()))
    }

    fn serialize_bytes(self, _value: &[u8]) -> SerdeResult<Self::Ok> {
        Err(SerdeValueError(
            "byte arrays cannot be table keys".to_owned(),
        ))
    }

    fn serialize_none(self) -> SerdeResult<Self::Ok> {
        Err(SerdeValueError("nil cannot be a table key".to_owned()))
    }

    fn serialize_some<T>(self, value: &T) -> SerdeResult<Self::Ok>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> SerdeResult<Self::Ok> {
        Err(SerdeValueError("unit cannot be a table key".to_owned()))
    }

    fn serialize_unit_struct(self, _name: &'static str) -> SerdeResult<Self::Ok> {
        Err(SerdeValueError(
            "unit struct cannot be a table key".to_owned(),
        ))
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> SerdeResult<Self::Ok> {
        Ok(OwnedKey::String(variant.to_owned()))
    }

    fn serialize_newtype_struct<T>(self, _name: &'static str, value: &T) -> SerdeResult<Self::Ok>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> SerdeResult<Self::Ok>
    where
        T: Serialize + ?Sized,
    {
        Err(SerdeValueError(
            "newtype variants cannot be table keys".to_owned(),
        ))
    }

    fn serialize_seq(self, _len: Option<usize>) -> SerdeResult<Self::SerializeSeq> {
        Err(SerdeValueError("sequences cannot be table keys".to_owned()))
    }

    fn serialize_tuple(self, _len: usize) -> SerdeResult<Self::SerializeTuple> {
        Err(SerdeValueError("tuples cannot be table keys".to_owned()))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> SerdeResult<Self::SerializeTupleStruct> {
        Err(SerdeValueError(
            "tuple structs cannot be table keys".to_owned(),
        ))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> SerdeResult<Self::SerializeTupleVariant> {
        Err(SerdeValueError(
            "tuple variants cannot be table keys".to_owned(),
        ))
    }

    fn serialize_map(self, _len: Option<usize>) -> SerdeResult<Self::SerializeMap> {
        Err(SerdeValueError("maps cannot be table keys".to_owned()))
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> SerdeResult<Self::SerializeStruct> {
        Err(SerdeValueError("structs cannot be table keys".to_owned()))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> SerdeResult<Self::SerializeStructVariant> {
        Err(SerdeValueError(
            "struct variants cannot be table keys".to_owned(),
        ))
    }
}

fn number_to_i64(value: f64) -> SerdeResult<i64> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(SerdeValueError(format!("expected integer, got {value}")));
    }
    if value < i64::MIN as f64 || value > i64::MAX as f64 {
        return Err(SerdeValueError(format!("{value} is outside i64 range")));
    }
    Ok(value as i64)
}

fn number_to_u64(value: f64) -> SerdeResult<u64> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(SerdeValueError(format!("expected integer, got {value}")));
    }
    if value < 0.0 || value > u64::MAX as f64 {
        return Err(SerdeValueError(format!("{value} is outside u64 range")));
    }
    Ok(value as u64)
}

fn number_to_i64_value(value: &OwnedValue) -> SerdeResult<i64> {
    match value {
        OwnedValue::Number(value) => number_to_i64(*value),
        value => Err(SerdeValueError(format!(
            "expected number, got {}",
            value.type_name()
        ))),
    }
}

fn number_to_u64_value(value: &OwnedValue) -> SerdeResult<u64> {
    match value {
        OwnedValue::Number(value) => number_to_u64(*value),
        value => Err(SerdeValueError(format!(
            "expected number, got {}",
            value.type_name()
        ))),
    }
}

fn integer_in_range<T, U>(value: U) -> SerdeResult<T>
where
    T: TryFrom<U>,
    U: Copy + fmt::Display,
{
    T::try_from(value).map_err(|_| {
        SerdeValueError(format!(
            "{value} is outside {} range",
            std::any::type_name::<T>()
        ))
    })
}
