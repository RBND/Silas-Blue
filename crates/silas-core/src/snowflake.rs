//! Serde serialization and deserialization helpers for Discord snowflake IDs (`u64`).
//!
//! Because JavaScript numbers lose precision above 2^53 - 1 (9,007,199,254,740,991),
//! Discord snowflake IDs (~19 digits) must be transferred as strings in JSON.
//! These helpers serialize `u64` as JSON strings and deserialize flexibly from either
//! JSON strings or JSON numbers.

use std::fmt;
use serde::de::{self, Deserializer, Visitor};
use serde::Serializer;

/// Serializes a `u64` snowflake as a JSON string.
pub fn serialize<S>(val: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&val.to_string())
}

/// Deserializes a `u64` snowflake from either a JSON string or an integer.
pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    struct SnowflakeVisitor;

    impl<'de> Visitor<'de> for SnowflakeVisitor {
        type Value = u64;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a Discord snowflake ID as a string or integer")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value < 0 {
                return Err(de::Error::custom("snowflake ID cannot be negative"));
            }
            Ok(value as u64)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let trimmed = value.trim();
            trimmed.parse::<u64>().map_err(de::Error::custom)
        }
    }

    deserializer.deserialize_any(SnowflakeVisitor)
}

/// Helpers for `Option<u64>` snowflakes.
pub mod opt {
    use super::*;

    pub fn serialize<S>(val: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match val {
            Some(id) => serializer.serialize_str(&id.to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct OptSnowflakeVisitor;

        impl<'de> Visitor<'de> for OptSnowflakeVisitor {
            type Value = Option<u64>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an optional Discord snowflake ID as a string, integer, or null")
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(None)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                super::deserialize(deserializer).map(Some)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(None)
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Some(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value < 0 {
                    return Err(de::Error::custom("snowflake ID cannot be negative"));
                }
                Ok(Some(value as u64))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let trimmed = value.trim();
                if trimmed.is_empty() || trimmed == "null" {
                    Ok(None)
                } else {
                    trimmed.parse::<u64>().map(Some).map_err(de::Error::custom)
                }
            }
        }

        deserializer.deserialize_any(OptSnowflakeVisitor)
    }
}

/// Helpers for `Vec<u64>` snowflakes.
pub mod vec {
    use super::*;
    use serde::de::SeqAccess;
    use serde::ser::SerializeSeq;

    pub fn serialize<S>(val: &[u64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(val.len()))?;
        for id in val {
            seq.serialize_element(&id.to_string())?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SnowflakeVecVisitor;

        impl<'de> Visitor<'de> for SnowflakeVecVisitor {
            type Value = Vec<u64>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a list of Discord snowflake IDs as strings or integers")
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Vec::new())
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Vec::new())
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let mut out = Vec::new();
                for part in value.split(',') {
                    let trimmed = part.trim();
                    if !trimmed.is_empty() {
                        let id = trimmed.parse::<u64>().map_err(de::Error::custom)?;
                        out.push(id);
                    }
                }
                Ok(out)
            }

            fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
            where
                S: SeqAccess<'de>,
            {
                let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                while let Some(elem) = seq.next_element::<serde_json::Value>()? {
                    match elem {
                        serde_json::Value::Number(n) => {
                            if let Some(u) = n.as_u64() {
                                out.push(u);
                            } else if let Some(i) = n.as_i64() {
                                if i >= 0 {
                                    out.push(i as u64);
                                }
                            }
                        }
                        serde_json::Value::String(s) => {
                            let trimmed = s.trim();
                            if !trimmed.is_empty() {
                                let id = trimmed.parse::<u64>().map_err(de::Error::custom)?;
                                out.push(id);
                            }
                        }
                        _ => {}
                    }
                }
                Ok(out)
            }
        }

        deserializer.deserialize_any(SnowflakeVecVisitor)
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct TestContainer {
        #[serde(with = "crate::snowflake")]
        single: u64,
        #[serde(default, with = "crate::snowflake::opt")]
        opt_some: Option<u64>,
        #[serde(default, with = "crate::snowflake::opt")]
        opt_none: Option<u64>,
        #[serde(default, with = "crate::snowflake::vec")]
        items: Vec<u64>,
    }

    #[test]
    fn test_deserialize_snowflake_from_strings() -> Result<(), Box<dyn std::error::Error>> {
        let json_data = r#"{
            "single": "1341857943265509426",
            "opt_some": "824319214248132758",
            "opt_none": "",
            "items": ["295767175426473984", "497607738227097600"]
        }"#;

        let parsed: TestContainer = serde_json::from_str(json_data)?;
        assert_eq!(parsed.single, 1341857943265509426);
        assert_eq!(parsed.opt_some, Some(824319214248132758));
        assert_eq!(parsed.opt_none, None);
        assert_eq!(parsed.items, vec![295767175426473984, 497607738227097600]);
        Ok(())
    }

    #[test]
    fn test_deserialize_snowflake_from_numbers() -> Result<(), Box<dyn std::error::Error>> {
        let json_data = r#"{
            "single": 11223344,
            "opt_some": 55667788,
            "opt_none": null,
            "items": [1122, 3344]
        }"#;

        let parsed: TestContainer = serde_json::from_str(json_data)?;
        assert_eq!(parsed.single, 11223344);
        assert_eq!(parsed.opt_some, Some(55667788));
        assert_eq!(parsed.opt_none, None);
        assert_eq!(parsed.items, vec![1122, 3344]);
        Ok(())
    }

    #[test]
    fn test_serialize_snowflakes() -> Result<(), Box<dyn std::error::Error>> {
        let obj = TestContainer {
            single: 1341857943265509426,
            opt_some: Some(824319214248132758),
            opt_none: None,
            items: vec![295767175426473984],
        };

        let serialized = serde_json::to_string(&obj)?;
        assert!(serialized.contains(r#""single":"1341857943265509426""#));
        assert!(serialized.contains(r#""opt_some":"824319214248132758""#));
        assert!(serialized.contains(r#""opt_none":null"#));
        assert!(serialized.contains(r#""items":["295767175426473984"]"#));
        Ok(())
    }
}
