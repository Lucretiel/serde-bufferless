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

// This is what would be generated on a derive(Deserialize) for this type
#[derive(Debug)]
struct Outer {
    float: f32,
    boolean: bool,

    //#[serde(flatten)]
    inner: Inner,
}

impl<'de> Deserialize<'de> for Outer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[allow(non_camel_case_types)]
        enum Field {
            float,
            boolean,
        }

        struct Capture {
            float: Option<f32>,
            boolean: Option<bool>,
        }

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
            fn send_value<D>(&mut self, token: Self::Token, value: D) -> Result<(), D::Error>
            where
                D: serde::de::Deserializer<'de>,
            {
                match token {
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
