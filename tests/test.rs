use pretty_assertions::assert_eq;
use serde::{Deserialize, Deserializer};
use serde_bufferless::private::flatten::{FlattenDeserializer, KeyCapture};

#[derive(Debug, PartialEq, Deserialize)]
struct Inner {
    integer: i32,
    string: String,

    /// `before` is captured by `Outer` and never reaches this point
    #[serde(default)]
    before: f32,
}

#[derive(Debug, PartialEq)]
struct Outer {
    // #[serde(default)]
    before: Option<f32>,

    //#[serde(flatten)]
    inner: Inner,

    // #[serde(default)]
    after: Option<bool>,
}
/////////////////////////////////////////////////////////////////////////
// This is what would be generated on a derive(Deserialize) for this type
/////////////////////////////////////////////////////////////////////////
impl<'de> Deserialize<'de> for Outer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // The Field enum is generated for each non-flatten field
        #[allow(non_camel_case_types)]
        enum Field {
            before,
            after,
        }

        // The Capture struct is generated, containing an Option for each
        // non-capture field
        struct Capture {
            before: Option<Option<f32>>,
            after: Option<Option<bool>>,
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
                    b"before" => Some(Field::before),
                    b"after" => Some(Field::after),
                    _ => None,
                }
            }

            #[inline]
            fn send_value<D>(&mut self, field: Self::Token, value: D) -> Result<(), D::Error>
            where
                D: Deserializer<'de>,
            {
                match field {
                    Field::before => self.before = Some(Deserialize::deserialize(value)?),
                    Field::after => self.after = Some(Deserialize::deserialize(value)?),
                }

                Ok(())
            }

            fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(formatter, "struct Outer")
            }
        }

        let mut capture = Capture {
            before: None,
            after: None,
        };

        // After the `Capture` is created, we use a `FlattenDeserializer` to
        // deserialize the flattened field. The `FlattenDeserializer` will
        // populate `capture` while this is happening
        let inner = Deserialize::deserialize(FlattenDeserializer::new(deserializer, &mut capture))?;

        let before = capture
            .before
            // This code should generated without `#[serde(default)]`
            // .ok_or_else(|| de::Error::missing_field("before"))?;
            .unwrap_or_default();

        let after = capture
            .after
            // This code should generated without `#[serde(default)]`
            // .ok_or_else(|| de::Error::missing_field("after"))?;
            .unwrap_or_default();

        Ok(Self {
            before,
            inner,
            after,
        })
    }
}

#[test]
fn one_field() {
    let data: Outer = serde_json::from_str(
        r#"{
            "_1": 1,
            "integer": 10,
            "_2": 2,
            "before": 10.5,
            "_3": 3,
            "string": "hello",
            "_4": 4,
            "after": true,
            "_5": 5
        }"#,
    )
    .expect("failed to parse JSON");

    assert_eq!(
        data,
        Outer {
            before: Some(10.5),
            inner: Inner {
                integer: 10,
                string: "hello".to_string(),
                before: 0.0,
            },
            after: Some(true),
        }
    );
}
