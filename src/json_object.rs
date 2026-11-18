use std::{fmt, marker::PhantomData};

use serde::{
    Deserialize, Deserializer,
    de::{
        self, IgnoredAny, MapAccess, SeqAccess, Unexpected, Visitor, value::MapAccessDeserializer,
    },
};

/// A named-field request model. Serde's default struct decoder also accepts positional arrays.
pub(crate) struct Object<T>(pub(crate) T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
            type Value = Object<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                // Pass entries directly to the model so repeated field names remain visible.
                T::deserialize(MapAccessDeserializer::new(map)).map(Object)
            }

            fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Self::Value, S::Error> {
                // Let malformed arrays report a syntax error before rejecting their shape.
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Err(de::Error::invalid_type(Unexpected::Seq, &self))
            }
        }

        deserializer.deserialize_any(ObjectVisitor(PhantomData))
    }
}

pub(crate) fn optional<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<Object<T>>::deserialize(deserializer).map(|value| value.map(|Object(item)| item))
}

pub(crate) fn list<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<T>, D::Error> {
    Vec::<Object<T>>::deserialize(deserializer)
        .map(|values| values.into_iter().map(|Object(item)| item).collect())
}
