/*!
Components that would be added to serde's private module to support
bufferless deserialization
*/

pub mod flatten;

use std::marker::PhantomData;

use serde::{de, forward_to_deserialize_any};

/// Struct to fuse a MapAccess or a SeqAccess
struct FusedAccess<A> {
    access: Option<A>,
}

impl<A> FusedAccess<A> {
    pub fn new(access: A) -> Self {
        Self {
            access: Some(access),
        }
    }

    fn next_item<T, E>(
        &mut self,
        op: impl FnOnce(&mut A) -> Result<Option<T>, E>,
    ) -> Result<Option<T>, E> {
        match self.access {
            None => Ok(None),
            Some(ref mut access) => op(access).map(|item| match item {
                None => {
                    self.access = None;
                    None
                }
                Some(item) => Some(item),
            }),
        }
    }
}

impl<'de, A: de::SeqAccess<'de>> de::SeqAccess<'de> for FusedAccess<A> {
    type Error = A::Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: de::DeserializeSeed<'de>,
    {
        self.next_item(|access| access.next_element_seed(seed))
    }
}

impl<'de, A: de::MapAccess<'de>> de::MapAccess<'de> for FusedAccess<A> {
    type Error = A::Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: de::DeserializeSeed<'de>,
    {
        self.next_item(|access| access.next_key_seed(seed))
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: de::DeserializeSeed<'de>,
    {
        self.access
            .as_mut()
            .expect("called next_value_seed after next_key_seed returned None")
            .next_value_seed(seed)
    }

    fn next_entry_seed<K, V>(
        &mut self,
        key: K,
        value: V,
    ) -> Result<Option<(K::Value, V::Value)>, Self::Error>
    where
        K: de::DeserializeSeed<'de>,
        V: de::DeserializeSeed<'de>,
    {
        self.next_item(|access| access.next_entry_seed(key, value))
    }
}

/// Additional versions of IntoDeserializer
struct EnumDeserializer<T> {
    value: T,
}

impl<'de, T> EnumDeserializer<T>
where
    T: de::EnumAccess<'de>,
{
    pub fn new(value: T) -> Self {
        Self { value }
    }
}

impl<'de, T> de::Deserializer<'de> for EnumDeserializer<T>
where
    T: de::EnumAccess<'de>,
{
    type Error = T::Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        visitor.visit_enum(self.value)
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

struct NewtypeDeserializer<T> {
    deserializer: T,
}

impl<'de, T> NewtypeDeserializer<T>
where
    T: de::Deserializer<'de>,
{
    pub fn new(deserializer: T) -> Self {
        Self { deserializer }
    }
}

impl<'de, T> de::Deserializer<'de> for NewtypeDeserializer<T>
where
    T: de::Deserializer<'de>,
{
    type Error = T::Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        visitor.visit_newtype_struct(self.deserializer)
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}
pub struct SomeDeserializer<T> {
    deserializer: T,
}

impl<'de, T> SomeDeserializer<T>
where
    T: de::Deserializer<'de>,
{
    pub fn new(deserializer: T) -> Self {
        Self { deserializer }
    }
}

impl<'de, T> de::Deserializer<'de> for SomeDeserializer<T>
where
    T: de::Deserializer<'de>,
{
    type Error = T::Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        visitor.visit_some(self.deserializer)
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

pub struct ByteBufDeserializer<E> {
    buf: Vec<u8>,
    phantom: PhantomData<E>,
}

impl<E> ByteBufDeserializer<E> {
    pub fn new(buf: Vec<u8>) -> Self {
        Self {
            buf,
            phantom: PhantomData,
        }
    }
}

impl<'de, E> de::Deserializer<'de> for ByteBufDeserializer<E>
where
    E: de::Error,
{
    type Error = E;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        visitor.visit_byte_buf(self.buf)
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}
