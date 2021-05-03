// copy-pasted and modified serde_json code to be able to handle
// JSON with repeated field names. At first I attempted to use
// tshark's --no-duplicate-keys but i hit https://gitlab.com/wireshark/wireshark/-/issues/17369
// and so I decide to handle the duplicated fields by hand after all

use serde::de;
use serde::de::DeserializeSeed;
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::Number;
use std::fmt;
use std::fmt::Debug;

/// Represents any valid JSON value.
///
/// See the [`serde_json::value` module documentation](self) for usage examples.
#[derive(Clone, Eq, PartialEq)]
pub enum MultiVal {
    /// Represents a JSON null value.
    ///
    /// ```
    /// # use serde_json::json;
    /// #
    /// let v = json!(null);
    /// ```
    Null,

    /// Represents a JSON boolean.
    ///
    /// ```
    /// # use serde_json::json;
    /// #
    /// let v = json!(true);
    /// ```
    Bool(bool),

    /// Represents a JSON number, whether integer or floating point.
    ///
    /// ```
    /// # use serde_json::json;
    /// #
    /// let v = json!(12.5);
    /// ```
    Number(Number),

    /// Represents a JSON string.
    ///
    /// ```
    /// # use serde_json::json;
    /// #
    /// let v = json!("a string");
    /// ```
    String(String),

    /// Represents a JSON array.
    ///
    /// ```
    /// # use serde_json::json;
    /// #
    /// let v = json!(["an", "array"]);
    /// ```
    Array(Vec<MultiVal>),

    /// Represents a JSON object.
    ///
    /// By default the map is backed by a BTreeMap. Enable the `preserve_order`
    /// feature of serde_json to use IndexMap instead, which preserves
    /// entries in the order they are inserted into the map. In particular, this
    /// allows JSON data to be deserialized into a MultiVal and serialized to a
    /// string while retaining the order of map keys in the input.
    ///
    /// ```
    /// # use serde_json::json;
    /// #
    /// let v = json!({ "an": "object" });
    /// ```
    Object(Vec<(String, MultiVal)>),
}

impl Debug for MultiVal {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            MultiVal::Null => formatter.debug_tuple("Null").finish(),
            MultiVal::Bool(v) => formatter.debug_tuple("Bool").field(&v).finish(),
            MultiVal::Number(ref v) => Debug::fmt(v, formatter),
            MultiVal::String(ref v) => formatter.debug_tuple("String").field(v).finish(),
            MultiVal::Array(ref v) => {
                formatter.write_str("Array(")?;
                Debug::fmt(v, formatter)?;
                formatter.write_str(")")
            }
            MultiVal::Object(ref v) => {
                formatter.write_str("Object(")?;
                Debug::fmt(v, formatter)?;
                formatter.write_str(")")
            }
        }
    }
}

impl<'de> Deserialize<'de> for MultiVal {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<MultiVal, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct MultiValVisitor;

        impl<'de> Visitor<'de> for MultiValVisitor {
            type Value = MultiVal;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("any valid JSON value")
            }

            #[inline]
            fn visit_bool<E>(self, value: bool) -> Result<MultiVal, E> {
                Ok(MultiVal::Bool(value))
            }

            #[inline]
            fn visit_i64<E>(self, value: i64) -> Result<MultiVal, E> {
                Ok(MultiVal::Number(value.into()))
            }

            #[inline]
            fn visit_u64<E>(self, value: u64) -> Result<MultiVal, E> {
                Ok(MultiVal::Number(value.into()))
            }

            #[inline]
            fn visit_f64<E>(self, value: f64) -> Result<MultiVal, E> {
                Ok(Number::from_f64(value).map_or(MultiVal::Null, MultiVal::Number))
            }

            #[inline]
            fn visit_str<E>(self, value: &str) -> Result<MultiVal, E>
            where
                E: serde::de::Error,
            {
                self.visit_string(String::from(value))
            }

            #[inline]
            fn visit_string<E>(self, value: String) -> Result<MultiVal, E> {
                Ok(MultiVal::String(value))
            }

            #[inline]
            fn visit_none<E>(self) -> Result<MultiVal, E> {
                Ok(MultiVal::Null)
            }

            #[inline]
            fn visit_some<D>(self, deserializer: D) -> Result<MultiVal, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                Deserialize::deserialize(deserializer)
            }

            #[inline]
            fn visit_unit<E>(self) -> Result<MultiVal, E> {
                Ok(MultiVal::Null)
            }

            #[inline]
            fn visit_seq<V>(self, mut visitor: V) -> Result<MultiVal, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let mut vec = Vec::new();

                while let Some(elem) = r#try!(visitor.next_element()) {
                    vec.push(elem);
                }

                Ok(MultiVal::Array(vec))
            }

            fn visit_map<V>(self, mut visitor: V) -> Result<MultiVal, V::Error>
            where
                V: MapAccess<'de>,
            {
                match visitor.next_key_seed(KeyClassifier)? {
                    Some(KeyClass::RawValue) => {
                        panic!("raw_value not handled");
                    }
                    Some(KeyClass::Number) => {
                        // let number: serde_json::number::NumberFromString = visitor.next_value()?;
                        // Ok(MultiVal::Number(number.value))
                        let number: Number = visitor.next_value()?;
                        Ok(MultiVal::Number(number))
                    }
                    Some(KeyClass::Map(first_key)) => {
                        let mut values = vec![];

                        values.push((first_key, r#try!(visitor.next_value())));
                        while let Some((key, value)) = r#try!(visitor.next_entry()) {
                            values.push((key, value));
                        }

                        Ok(MultiVal::Object(values))
                    }
                    None => Ok(MultiVal::Object(vec![])),
                }
            }
        }

        deserializer.deserialize_any(MultiValVisitor)
    }
}

struct KeyClassifier;

enum KeyClass {
    Map(String),
    Number,
    RawValue,
}

impl<'de> DeserializeSeed<'de> for KeyClassifier {
    type Value = KeyClass;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(self)
    }
}

const NUMBER_TOKEN: &str = "$serde_json::private::Number";
const RAW_TOKEN: &str = "$serde_json::private::RawValue";

impl<'de> Visitor<'de> for KeyClassifier {
    type Value = KeyClass;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a string key")
    }

    fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match s {
            NUMBER_TOKEN => Ok(KeyClass::Number),
            RAW_TOKEN => Ok(KeyClass::RawValue),
            _ => Ok(KeyClass::Map(s.to_owned())),
        }
    }

    fn visit_string<E>(self, s: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        match s.as_str() {
            NUMBER_TOKEN => Ok(KeyClass::Number),
            RAW_TOKEN => Ok(KeyClass::RawValue),
            _ => Ok(KeyClass::Map(s)),
        }
    }
}
