//! The 8-variant value model and the `Request`/`Response` frames.
//!
//! Externally-tagged encoding (rmp-serde default): unit variants serialize
//! as a bare string (`"Null"`), payload variants as a single-key map
//! (`{"Int": 42}`). `Response.result` is a serde `Result`, so a successful
//! string reply nests two one-key maps: `{"Ok": {"Str": "PONG"}}` — pinned
//! by the conformance corpus.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// The wire value model (WIRE-002) — byte-compatible with the value
/// models the family shipped before Thunder, by construction.
///
/// `Map` is an insertion-ordered pair list because keys may be any value.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Value {
    /// SQL NULL / nil.
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// Raw bytes, refcounted. Emitted as MessagePack **bin** (WIRE-010); the
    /// legacy int-array form decodes too (WIRE-011).
    ///
    /// The payload is `Arc<[u8]>` rather than `Vec<u8>` so a decoded value can
    /// move into a product's store, and a stored buffer can reach the encoder,
    /// as a **refcount bump instead of a memcpy** — in both directions. With an
    /// owned `Vec`, a server paid one full copy of the payload per read and per
    /// write, worst exactly where a binary protocol is supposed to win: large
    /// values and the raw-LE-f32 embeddings this wire exists to carry.
    ///
    /// **The wire is unchanged.** The emitted form is still MessagePack `bin`
    /// and the legacy int-array form is still accepted, so no corpus vector
    /// moves and no other language lane is affected — only the Rust type.
    Bytes(#[serde(with = "arc_bytes")] Arc<[u8]>),
    Str(String),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
}

/// Serde adapter for [`Value::Bytes`]: emits MessagePack **bin** (WIRE-010)
/// and accepts both `bin` and the legacy int-array form (WIRE-011), exactly
/// as `serde_bytes` does for `Vec<u8>` — the refcounted payload is an
/// in-process detail the wire never sees.
mod arc_bytes {
    use std::sync::Arc;

    use serde::{Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        bytes: &Arc<[u8]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serde_bytes::serialize(&**bytes, serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<[u8]>, D::Error> {
        // One allocation on decode, as before; what is saved is every copy
        // *after* it — the value can now be shared instead of cloned.
        let bytes: Vec<u8> = serde_bytes::deserialize(deserializer)?;
        Ok(Arc::from(bytes))
    }
}

impl Value {
    /// Extract the inner string slice.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Extract bytes (also accepts `Str` as UTF-8 bytes).
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(b) => Some(b),
            Self::Str(s) => Some(s.as_bytes()),
            _ => None,
        }
    }

    /// The shared buffer behind [`Value::Bytes`], for the zero-copy path.
    ///
    /// `Arc::clone` on the result is a refcount bump: a product can put the
    /// decoded payload straight into its store without copying it.
    pub fn as_shared_bytes(&self) -> Option<&Arc<[u8]>> {
        match self {
            Self::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Consume the value and take its shared buffer — no copy, no refcount
    /// bump beyond the move itself.
    pub fn into_shared_bytes(self) -> Option<Arc<[u8]>> {
        match self {
            Self::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Build a `Bytes` value from anything that can become a shared buffer.
    pub fn bytes(buffer: impl Into<Arc<[u8]>>) -> Self {
        Self::Bytes(buffer.into())
    }

    /// Extract an integer.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Extract a float (accepts `Int` widened to `f64`).
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Extract a bool.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Extract the array items.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    /// Extract the map pairs.
    pub fn as_map(&self) -> Option<&[(Value, Value)]> {
        match self {
            Self::Map(pairs) => Some(pairs.as_slice()),
            _ => None,
        }
    }

    /// Look up a string key in a `Map` value.
    pub fn map_get(&self, key: &str) -> Option<&Value> {
        self.as_map()?
            .iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .map(|(_, v)| v)
    }

    /// True for `Value::Null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}
impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Self::Int(i)
    }
}
impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Self::Float(f)
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Self::Str(s)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self::Str(s.to_owned())
    }
}
impl From<Vec<u8>> for Value {
    /// Copies once, at the boundary — use [`Value::from`] on an `Arc<[u8]>`
    /// (or [`Value::bytes`]) when the caller already holds a shared buffer.
    fn from(b: Vec<u8>) -> Self {
        Self::Bytes(Arc::from(b))
    }
}
impl From<Arc<[u8]>> for Value {
    /// The zero-copy path: a refcount bump, no payload copy.
    fn from(b: Arc<[u8]>) -> Self {
        Self::Bytes(b)
    }
}
impl From<&[u8]> for Value {
    fn from(b: &[u8]) -> Self {
        Self::Bytes(Arc::from(b))
    }
}
impl From<Vec<Value>> for Value {
    fn from(items: Vec<Value>) -> Self {
        Self::Array(items)
    }
}

/// One RPC request (WIRE-001). `id` is client-chosen and echoed back;
/// many requests multiplex over one connection. Serialized as an array
/// (WIRE-012); map-shaped requests decode too (WIRE-013).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Request {
    pub id: u32,
    pub command: String,
    pub args: Vec<Value>,
}

/// One RPC response (WIRE-001). `result` is `Ok(value)` or an error
/// string; v1 carries no structured error object — conventions are
/// prefix-based and profile-driven (WIRE-040).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Response {
    pub id: u32,
    pub result: Result<Value, String>,
}

impl Response {
    /// Success response.
    pub fn ok(id: u32, value: Value) -> Self {
        Self {
            id,
            result: Ok(value),
        }
    }

    /// Error response with the verbatim error string.
    pub fn err(id: u32, message: impl Into<String>) -> Self {
        Self {
            id,
            result: Err(message.into()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod bytes_sharing_tests {
    use super::*;

    /// The property this type change exists for: a decoded payload can be
    /// shared without copying it. Pinned as a test so a future refactor back
    /// to an owned `Vec` cannot silently reintroduce the memcpy the Synap
    /// adoption reported (GH #1).
    #[test]
    fn bytes_are_shared_not_copied() {
        let buffer: Arc<[u8]> = Arc::from(vec![7u8; 4096]);
        let value = Value::from(Arc::clone(&buffer));

        // Reading the payload out for a store is a refcount bump, not a copy.
        let taken = value.into_shared_bytes().unwrap();
        assert_eq!(Arc::strong_count(&buffer), 2, "shared, not cloned");
        assert!(
            Arc::ptr_eq(&buffer, &taken),
            "the very same allocation must come back out"
        );
    }

    /// The read direction: a stored buffer reaches the encoder without a copy.
    #[test]
    fn a_stored_buffer_reaches_a_value_without_copying() {
        let stored: Arc<[u8]> = Arc::from(vec![1u8, 2, 3]);
        let value = Value::bytes(Arc::clone(&stored));
        let inside = value.as_shared_bytes().unwrap();
        assert!(Arc::ptr_eq(&stored, inside));
    }

    /// And the wire is unchanged: the shared payload still round-trips, and
    /// still emits MessagePack `bin`.
    #[test]
    fn sharing_does_not_change_the_wire() {
        let value = Value::bytes(vec![1u8, 2, 3, 255]);
        let encoded = rmp_serde::to_vec(&value).unwrap();
        // 0xc4 is MessagePack bin8. Its position depends on the enum tagging,
        // so assert it is present rather than guessing an offset — what
        // matters is that the payload is bin and not an int array (WIRE-010).
        assert!(
            encoded.contains(&0xc4),
            "still emitted as bin, not an int array: {encoded:02x?}"
        );
        let decoded: Value = rmp_serde::from_slice(&encoded).unwrap();
        assert_eq!(decoded, value);
    }
}
