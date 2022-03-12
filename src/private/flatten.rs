/*!
Components that would be added to serde's private module to support
bufferless deserialization of structs with `#[serde(flatten)]` fields. This would
only work for structs with a single such field; it's impossible in the
general case to deserialize a struct with more than one `#[serde(flatten)]` field
without buffering.

This module provides a deserializer, [`FlattenDeserializer`], which adapts an
incoming deserializer. The [`FlattenDeserializer`] is used to deserialize the
inner, flattened type (we'll call that `F`). In the course of doing so, it
first sends keys it sees to a type implementing [`KeyCapture`]; this type
represents the other, non-flattened fields of the outer struct. [`KeyCapture`]
can indicate if it "wants" a key or not; it is sent a value for every key it
wants. Keys it doesn't want are then sent to `F` for ordinary deserialization.
*/

use core::fmt;

use serde::{de, forward_to_deserialize_any, serde_if_integer128, Deserialize};

use super::{EnumDeserializer, FusedAccess, NewtypeDeserializer, SomeDeserializer};

pub trait KeyCapture<'de> {
    type Token;

    /// Send a key into the KeyCapture, If this method returns a token, it
    /// means that has *accepted* the key, and a value should be provided to
    /// send_value with that token. Otherwise, the key was rejected, and can
    /// be passed into the visitor for the inner flattened struct.
    ///
    /// Because struct keys are only ever strings or byte slices when `flatten`
    /// is involved, we only need to have this version accepting a byte slice.
    ///
    /// Because the only thing we do with the key in practice is check it
    /// against a list of struct fields, this method doesn't ever return an
    /// error
    #[must_use]
    fn try_send_key(&mut self, key: &[u8]) -> Option<Self::Token>;

    /// Send a value into the KeyCapture. This should be called anytime
    /// try_send_key returns a token.
    ///
    /// This method always stores the deserialized value locally, inside of
    /// itself, so it never returns a value.
    fn send_value<D>(&mut self, token: Self::Token, value: D) -> Result<(), D::Error>
    where
        D: de::Deserializer<'de>;

    /// A KeyCapture fills a similar role as a Visitor, representing a
    /// destination for data to be deserialized, so it provides an expecting
    /// as well.
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result;
}

/// A [`FlattenDeserializer`] assists with deserializing a struct with a single
/// `#[serde(flatten)]` field. It is used to deserialize the inner flattened
/// value, but while running, it also captures the the other fields into
/// `capture`, only forwarding them to the flattened value of `capture` doesn't
/// want them.
pub struct FlattenDeserializer<D, C> {
    deserializer: D,
    capture: C,
}

impl<'de, D, C> FlattenDeserializer<D, C>
where
    D: de::Deserializer<'de>,
    C: KeyCapture<'de>,
{
    pub fn new(deserializer: D, capture: C) -> Self {
        Self {
            deserializer,
            capture,
        }
    }
}

impl<'de, D, C> de::Deserializer<'de> for FlattenDeserializer<D, C>
where
    D: de::Deserializer<'de>,
    C: KeyCapture<'de>,
{
    type Error = D::Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserializer.deserialize_map(FlattenVisitor {
            visitor,
            capture: self.capture,
        })
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserializer.deserialize_ignored_any(visitor)
    }
}

struct FlattenVisitor<V, C> {
    visitor: V,
    capture: C,
}

impl<'de, V, C> de::Visitor<'de> for FlattenVisitor<V, C>
where
    V: de::Visitor<'de>,
    C: KeyCapture<'de>,
{
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        self.capture.expecting(formatter)
    }

    fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
    where
        A: de::MapAccess<'de>,
    {
        // Create a `map` adapter to send to self.visitor. This adapter is
        // where we will take care of first trying to send keys to `capture`.
        // Additionally ensure that the `map` is drained after `visitor` is
        // done with it.
        let mut map = FlattenMapAccess {
            map: FusedAccess::new(map),
            capture: self.capture,
        };

        let value = self.visitor.visit_map(&mut map)?;

        // Drain remaining values from the map. This ensures that, if the
        // visitor left any behind, they're still propagated to the capture.
        let _ = de::IgnoredAny::deserialize(de::value::MapAccessDeserializer::new(&mut map))?;

        Ok(value)
    }
}

struct FlattenMapAccess<M, C> {
    map: FusedAccess<M>,
    capture: C,
}

impl<'de, M, C> de::MapAccess<'de> for FlattenMapAccess<M, C>
where
    M: de::MapAccess<'de>,
    C: KeyCapture<'de>,
{
    type Error = M::Error;

    fn next_key_seed<K>(&mut self, mut seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: de::DeserializeSeed<'de>,
    {
        let capture = &mut self.capture;
        // This is the interesting part. The inner type's visitor has requested
        // a key. We get one from self.map, and first try sending it to capture,
        // only returning it if the capture didn't want it. We do this
        // repeatedly until we can return something.
        loop {
            seed = match self.map.next_key_seed(FlattenKeySeed { seed, capture })? {
                None => return Ok(None),
                Some(FlattenKeySeedOutcome::Rejected(value)) => return Ok(Some(value)),
                Some(FlattenKeySeedOutcome::Accepted(seed, token)) => {
                    self.map
                        .next_value_seed(FlattenValueSeed { token, capture })?;
                    seed
                }
            }
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: de::DeserializeSeed<'de>,
    {
        self.map.next_value_seed(seed)
    }
}

struct FlattenKeySeed<'a, S, C> {
    seed: S,
    capture: &'a mut C,
}

enum FlattenKeySeedOutcome<'de, T, S: de::DeserializeSeed<'de>> {
    /// If the key was accepted by capture, return the unused seed, as well as
    /// the token
    Accepted(S, T),

    /// If the key was rejected by `capture`, it was instead deserialized by
    /// seed. Return the produced value.
    Rejected(S::Value),
}

impl<'a, 'de, S, C> de::DeserializeSeed<'de> for FlattenKeySeed<'a, S, C>
where
    S: de::DeserializeSeed<'de>,
    C: KeyCapture<'de>,
{
    type Value = FlattenKeySeedOutcome<'de, C::Token, S>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_identifier(self)
    }
}

impl<'a, 'de, S, C> FlattenKeySeed<'a, S, C>
where
    S: de::DeserializeSeed<'de>,
    C: KeyCapture<'de>,
{
    fn send_to_seed<D>(
        self,
        deserializer: D,
    ) -> Result<FlattenKeySeedOutcome<'de, C::Token, S>, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        self.seed
            .deserialize(deserializer)
            .map(FlattenKeySeedOutcome::Rejected)
    }

    fn send_into_to_seed<T, E>(
        self,
        deserializer: T,
    ) -> Result<FlattenKeySeedOutcome<'de, C::Token, S>, E>
    where
        T: de::IntoDeserializer<'de, E>,
        E: de::Error,
    {
        self.send_to_seed(deserializer.into_deserializer())
    }

    fn send_to_capture<T, D>(
        self,
        key: T,
        into_de: impl FnOnce(T) -> D,
    ) -> Result<FlattenKeySeedOutcome<'de, C::Token, S>, D::Error>
    where
        T: AsRef<[u8]>,
        D: de::Deserializer<'de>,
    {
        match self.capture.try_send_key(key.as_ref()) {
            Some(token) => Ok(FlattenKeySeedOutcome::Accepted(self.seed, token)),
            None => self.send_to_seed(into_de(key)),
        }
    }

    fn send_into_to_capture<T, E>(
        self,
        key: T,
    ) -> Result<FlattenKeySeedOutcome<'de, C::Token, S>, E>
    where
        T: AsRef<[u8]>,
        T: de::IntoDeserializer<'de, E>,
        E: de::Error,
    {
        self.send_to_capture(key, |key| key.into_deserializer())
    }
}

impl<'a, 'de, S, C> de::Visitor<'de> for FlattenKeySeed<'a, S, C>
where
    S: de::DeserializeSeed<'de>,
    C: KeyCapture<'de>,
{
    type Value = FlattenKeySeedOutcome<'de, C::Token, S>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "field identifier")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_seed(v)
    }

    fn visit_i8<E>(self, v: i8) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_seed(v)
    }

    fn visit_i16<E>(self, v: i16) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_seed(v)
    }

    fn visit_i32<E>(self, v: i32) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_seed(v)
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_seed(v)
    }

    serde_if_integer128! {
        fn visit_i128<E>(self, v: i128) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.send_into_to_seed(v)
        }

        fn visit_u128<E>(self, v: u128) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.send_into_to_seed(v)
        }
    }

    fn visit_u8<E>(self, v: u8) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_seed(v)
    }

    fn visit_u16<E>(self, v: u16) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_seed(v)
    }

    fn visit_u32<E>(self, v: u32) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_seed(v)
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_seed(v)
    }

    fn visit_f32<E>(self, v: f32) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_seed(v)
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_seed(v)
    }

    fn visit_char<E>(self, v: char) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_seed(v)
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_capture(v)
    }

    #[cfg(feature = "std")]
    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_capture(v)
    }

    fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_to_capture(v, de::value::BorrowedStrDeserializer::new)
    }

    fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_to_capture(v, de::value::BytesDeserializer::new)
    }

    #[cfg(feature = "std")]
    fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        use super::ByteBufDeserializer;

        self.send_to_capture(v, ByteBufDeserializer::new)
    }

    fn visit_borrowed_bytes<E>(self, v: &'de [u8]) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_to_capture(v, de::value::BorrowedBytesDeserializer::new)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_seed(())
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        self.send_to_seed(SomeDeserializer::new(deserializer))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.send_into_to_seed(())
    }

    fn visit_newtype_struct<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        self.send_to_seed(NewtypeDeserializer::new(deserializer))
    }

    fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        self.send_to_seed(de::value::SeqAccessDeserializer::new(seq))
    }

    fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
    where
        A: de::MapAccess<'de>,
    {
        self.send_to_seed(de::value::MapAccessDeserializer::new(map))
    }

    fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
    where
        A: de::EnumAccess<'de>,
    {
        self.send_to_seed(EnumDeserializer::new(data))
    }
}

struct FlattenValueSeed<'de, 'a, C: KeyCapture<'de>> {
    token: C::Token,
    capture: &'a mut C,
}

impl<'de, 'a, C> de::DeserializeSeed<'de> for FlattenValueSeed<'de, 'a, C>
where
    C: KeyCapture<'de>,
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        self.capture.send_value(self.token, deserializer)
    }
}
