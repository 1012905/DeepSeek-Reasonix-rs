//! Go `encoding/json`-compatible serialization helpers.
//!
//! Go's `json.Marshal` HTML-escapes `<`, `>`, `&`, U+2028 and U+2029 in
//! every string it emits. The reference implementation persists records
//! produced by `json.Marshal`, so byte-compatible output (golden
//! comparisons, digests, cross-implementation interop) requires the same
//! escaping. `serde_json` never emits those escapes on its own, so
//! [`marshal`] adds them after the fact: the escaped characters never appear
//! in JSON structure, only inside string values, which makes a
//! whole-document replacement safe.
//!
//! The `pub(crate)` helpers below reproduce Go's field-decode semantics for
//! the plain-struct types in this crate: JSON `null` decodes to the zero
//! value instead of an error, missing keys use the default, and a type
//! mismatch fails the whole decode (contract
//! `docs/contracts/provider-message.md` 0).

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;

/// Serialize `value` the way Go's `json.Marshal` would: compact, field
/// order from declaration, HTML-escaped strings.
pub fn marshal<T>(value: &T) -> Result<String, serde_json::Error>
where
    T: ?Sized + Serialize,
{
    Ok(escape_html(&serde_json::to_string(value)?))
}

/// Apply Go's HTML escaping to an already-serialized JSON document.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(c),
        }
    }
    out
}

/// Deserialize a plain Go struct field: JSON `null` becomes the zero value
/// instead of an error, a missing key uses the default, and a type mismatch
/// fails the whole decode.
pub fn null_default<'de, D, T>(de: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    let v = Option::<T>::deserialize(de)?;
    Ok(v.unwrap_or_default())
}

/// Deserialize a Go slice field with `omitempty`: `null` or a missing key
/// becomes an empty slice, `null` elements become zero values.
pub fn go_vec<'de, D, T>(de: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    let v = Option::<Vec<Option<T>>>::deserialize(de)?;
    Ok(v.unwrap_or_default()
        .into_iter()
        .map(|x| x.unwrap_or_default())
        .collect())
}

/// Deserialize a Go slice field without `omitempty` (nil marshals as
/// `null`): the field stays `Option` so `None` re-marshals as `null`
/// and `Some(vec![])` as `[]`.
pub fn go_opt_vec<'de, D, T>(de: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    let v = Option::<Vec<Option<T>>>::deserialize(de)?;
    Ok(v.map(|xs| xs.into_iter().map(|x| x.unwrap_or_default()).collect()))
}

/// Deserialize a `json.RawMessage` field with `omitempty`: a missing key is
/// `None`; an explicit `null` stays the raw literal `null`, exactly as Go
/// keeps the bytes.
pub fn go_raw<'de, D>(de: D) -> Result<Option<Box<RawValue>>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Some(Box::<RawValue>::deserialize(de)?))
}

/// Deserialize a `[]json.RawMessage` field with `omitempty`; elements keep
/// their raw bytes, including `null` literals.
pub fn go_raw_vec<'de, D>(de: D) -> Result<Vec<Box<RawValue>>, D::Error>
where
    D: Deserializer<'de>,
{
    let v = Option::<Vec<Box<RawValue>>>::deserialize(de)?;
    Ok(v.unwrap_or_default())
}

/// `omitempty` skip predicate for Go integer fields (zero is omitted).
pub fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}

/// `omitempty` skip predicate for Go bool fields (false is omitted).
pub fn is_zero_bool(v: &bool) -> bool {
    !*v
}

/// Declares a Go-style JSON struct.
///
/// `Serialize` derives normally with the caller-provided field
/// attributes. The generated `Deserialize` accepts only JSON objects:
/// Go's `encoding/json` rejects arrays for struct values, while
/// serde's derive would also accept a positional `[field, field, ...]`
/// list. A map decodes through a private mirror struct so every field keeps
/// its `#[serde]` semantics (`rename`, `default`,
/// `deserialize_with`, `skip_serializing_if`).
#[macro_export]
macro_rules! go_struct {
    (
        $(#[$outer:meta])*
        pub struct $name:ident : $wire:ident {
            $(
                $(#[$fmeta:meta])*
                pub $field:ident : $ty:ty
            ),* $(,)?
        }
    ) => {
        $(#[$outer])*
        #[derive(Debug, Clone, Default, serde::Serialize)]
        pub struct $name {
            $(
                $(#[$fmeta])*
                pub $field: $ty,
            )*
        }

        #[derive(serde::Deserialize)]
        struct $wire {
            $(
                $(#[$fmeta])*
                pub $field: $ty,
            )*
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                // serde_json's `deserialize_map` rejects every non-object
                // shape with an invalid-type error, matching Go.
                struct V;
                impl<'de> serde::de::Visitor<'de> for V {
                    type Value = $name;

                    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(f, "struct {} (a JSON object)", stringify!($name))
                    }

                    fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
                    where
                        A: serde::de::MapAccess<'de>,
                    {
                        let wire = <$wire as serde::Deserialize<'de>>::deserialize(
                            serde::de::value::MapAccessDeserializer::new(map),
                        )?;
                        Ok($name {
                            $( $field: wire.$field, )*
                        })
                    }
                }
                deserializer.deserialize_map(V)
            }
        }
    };
}
pub(crate) use go_struct;
// `#[macro_export]` above also re-exports the macro at the crate root for
// downstream crates (`reasonix_contract::go_struct`).
