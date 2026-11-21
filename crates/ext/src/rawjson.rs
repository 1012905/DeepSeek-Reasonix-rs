//! Raw JSON value that preserves the exact wire bytes.
//!
//! The protocol carries externalizable payloads, tool schemas, and error data
//! as opaque JSON. `serde_json::value::RawValue` keeps the bytes but has no
//! equality, so wire DTOs use this newtype: it decodes through `RawValue` and
//! re-serializes verbatim, while comparing by the retained byte string.

use serde::de::Deserializer;
use serde::ser::{self, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::fmt;

/// An opaque JSON value kept as its exact source text.
#[derive(Debug, Clone, Default)]
pub struct RawJson(String);

impl RawJson {
    /// Wraps a JSON document, rejecting malformed input.
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        text.parse()
    }

    /// The exact retained JSON text.
    pub fn get(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for RawJson {
    type Err = serde_json::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let _: &RawValue = serde_json::from_str(text)?;
        Ok(Self(text.to_owned()))
    }
}

impl PartialEq for RawJson {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for RawJson {}

impl fmt::Display for RawJson {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for RawJson {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let raw: &RawValue =
            serde_json::from_str(&self.0).map_err(|e| ser::Error::custom(e.to_string()))?;
        raw.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RawJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        Ok(Self(raw.get().to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_exact_bytes_and_compares() {
        let a = RawJson::parse("{\"b\":1,\"a\":2}").unwrap();
        let b = RawJson::parse("{\"b\":1,\"a\":2}").unwrap();
        let c = RawJson::parse("{\"a\":2,\"b\":1}").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(serde_json::to_string(&a).unwrap(), "{\"b\":1,\"a\":2}");
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(RawJson::parse("not json").is_err());
    }
}
