use anyhow::Context;
use serde::{de, Deserialize};
use serde_bufferless::private::flatten::{FlattenDeserializer, KeyCapture};

#[derive(Debug, Deserialize)]
struct Inner {
    integer: i32,
    string: String,

    /// `float` is captured by `Outer` and never reaches this point
    #[serde(default)]
    float: f32,
}

#[derive(Debug)]
struct Outer {
    float: f32,
    boolean: bool,

    //#[serde(flatten)]
    inner: Inner,
}
/////////////////////////////////////////////////////////////////////////
// This is what would be generated on a derive(Deserialize) for this type
/////////////////////////////////////////////////////////////////////////
impl<'de> Deserialize<'de> for Outer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // The Field enum is generated for each non-flatten field
        #[allow(non_camel_case_types)]
        enum Field {
            float,
            boolean,
        }

        // The Capture struct is generated, containing an Option for each
        // non-capture field
        struct Capture {
            float: Option<f32>,
            boolean: Option<bool>,
        }

        // KeyCapture is implemented such that `try_send_key` detects
        // non-flatten keys and returns the matching Field, if it matches,
        // and the `send_value` deserializes the relevant value based on the
        // `Field`. The methods are inlined so that control flow analysis
        // will notice the association between the return value of
        // `try_send_key` and the `match` in `send_value`
        impl<'de> KeyCapture<'de> for &mut Capture {
            type Token = Field;

            #[inline]
            fn try_send_key(&mut self, key: &[u8]) -> Option<Self::Token> {
                match key {
                    b"float" => Some(Field::float),
                    b"boolean" => Some(Field::boolean),
                    _ => None,
                }
            }

            #[inline]
            fn send_value<D>(&mut self, field: Self::Token, value: D) -> Result<(), D::Error>
            where
                D: serde::de::Deserializer<'de>,
            {
                match field {
                    Field::float => self.float = Some(Deserialize::deserialize(value)?),
                    Field::boolean => self.boolean = Some(Deserialize::deserialize(value)?),
                }

                Ok(())
            }

            fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(formatter, "struct Outer")
            }
        }

        let mut capture = Capture {
            float: None,
            boolean: None,
        };

        // After the `Capture` is created, we use a `FlattenDeserializer` to
        // deserialize the flattened field. The `FlattenDeserializer` will
        // populate `capture` while this is happening
        let inner = Deserialize::deserialize(FlattenDeserializer::new(deserializer, &mut capture))?;

        let float = capture
            .float
            .ok_or_else(|| de::Error::missing_field("float"))?;

        let boolean = capture
            .boolean
            .ok_or_else(|| de::Error::missing_field("boolean"))?;

        Ok(Self {
            float,
            boolean,
            inner,
        })
    }
}

#[test]
fn main() -> anyhow::Result<()> {
    let data: Outer = serde_json::from_str(
        r#"{
            "integer": 10,
            "float": 10.5,
            "string": "hello",
            "boolean": true
        }"#,
    )
    .context("failed to parse json")?;

    println!("{:#?}", data);

    Ok(())
}
