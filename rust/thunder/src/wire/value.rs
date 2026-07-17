//! The 8-variant value model and the `Request`/`Response` frames.
//!
//! Externally-tagged encoding (rmp-serde default): unit variants serialize
//! as a bare string (`"Null"`), payload variants as a single-key map
//! (`{"Int": 42}`). `Response.result` is a serde `Result`, so a successful
//! string reply nests two one-key maps: `{"Ok": {"Str": "PONG"}}` — pinned
//! by the conformance corpus.

use serde::{Deserialize, Serialize};

/// The wire value model — byte-compatible with `SynapValue` /
/// `NexusValue` / `VectorizerValue` (WIRE-002).
///
/// `Map` is an insertion-ordered pair list because keys may be any value.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Value {
    /// SQL NULL / nil.
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// Raw bytes. Emitted as MessagePack **bin** (WIRE-010); the legacy
    /// int-array form decodes too (WIRE-011) via `serde_bytes`' visitor.
    Bytes(#[serde(with = "serde_bytes")] Vec<u8>),
    Str(String),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
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
            Self::Bytes(b) => Some(b.as_slice()),
            Self::Str(s) => Some(s.as_bytes()),
            _ => None,
        }
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
    fn from(b: Vec<u8>) -> Self {
        Self::Bytes(b)
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
