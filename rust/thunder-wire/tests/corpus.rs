//! Conformance-corpus loader (TST-020): walks `conformance/vectors/` and
//! asserts every vector per its `mode`. Runs in the default test command —
//! never feature-gated (NFR-03).
//!
//! Mode semantics (TST-002, conformance/README.md):
//! - `bidirectional` — `encode(decoded) == frame` byte-exact AND
//!   `decode(frame) == decoded` structurally (floats by bit pattern).
//! - `decode-only`   — decode succeeds and equals `decoded`; the canonical
//!   encoding of `decoded` must NOT reproduce these legacy bytes.
//! - `stream`        — `frames` decode back-to-back, one frame per decode,
//!   consuming the buffer exactly.
//! - `incomplete`    — decoder asks for more bytes (no value, no error).
//! - `reject`        — decode fails with the named `error` class.
//!
//! `max_frame_bytes` (optional) overrides the 64 MiB default cap so the
//! at-cap / over-cap boundary is testable with real bytes.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use serde::Deserialize;
use thunder_wire::{
    decode_frame_with_limit, encode_frame, DecodeError, Request, Response, Value,
    DEFAULT_MAX_FRAME_BYTES,
};

#[derive(Deserialize, Debug)]
struct Vector {
    name: String,
    #[allow(dead_code)]
    group: String,
    mode: String,
    frame_hex: String,
    #[serde(default)]
    decoded: Option<Decoded>,
    #[serde(default)]
    frames: Option<Vec<Decoded>>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    max_frame_bytes: Option<usize>,
    #[allow(dead_code)]
    notes: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Decoded {
    Request {
        id: u32,
        command: String,
        args: Vec<Node>,
    },
    Response {
        id: u32,
        #[serde(default)]
        ok: Option<Node>,
        #[serde(default)]
        err: Option<String>,
    },
}

/// One `decoded` value node: `{type, value}` plus an optional `bits` field
/// for floats — the u64 IEEE-754 bit pattern in hex, required for NaN and
/// -0.0 where numeric equality cannot pin the wire bytes.
#[derive(Deserialize, Debug)]
struct Node {
    #[serde(rename = "type")]
    ty: String,
    #[serde(default)]
    value: serde_yaml::Value,
    #[serde(default)]
    bits: Option<String>,
}

fn node_to_value(n: &Node) -> Value {
    match n.ty.as_str() {
        "null" => Value::Null,
        "bool" => Value::Bool(n.value.as_bool().unwrap()),
        "int" => Value::Int(n.value.as_i64().unwrap()),
        "float" => match &n.bits {
            Some(bits) => Value::Float(f64::from_bits(u64::from_str_radix(bits, 16).unwrap())),
            None => Value::Float(n.value.as_f64().unwrap()),
        },
        "str" => Value::Str(n.value.as_str().unwrap().to_owned()),
        "bytes" => Value::Bytes(parse_hex(n.value.as_str().unwrap())),
        "array" => Value::Array(
            n.value
                .as_sequence()
                .unwrap()
                .iter()
                .map(|item| node_to_value(&yaml_node(item)))
                .collect(),
        ),
        "map" => Value::Map(
            n.value
                .as_sequence()
                .unwrap()
                .iter()
                .map(|pair| {
                    let kv = pair.as_sequence().unwrap();
                    assert_eq!(kv.len(), 2, "map entry must be a [key, value] pair");
                    (
                        node_to_value(&yaml_node(&kv[0])),
                        node_to_value(&yaml_node(&kv[1])),
                    )
                })
                .collect(),
        ),
        other => panic!("unknown corpus node type: {other}"),
    }
}

fn yaml_node(v: &serde_yaml::Value) -> Node {
    serde_yaml::from_value(v.clone()).expect("nested node must parse")
}

fn parse_hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

// ── Structural equality (floats by bit pattern — NaN/-0.0 safe) ─────────────

fn values_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        (Value::Array(xs), Value::Array(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| values_eq(x, y))
        }
        (Value::Map(xs), Value::Map(ys)) => {
            xs.len() == ys.len()
                && xs
                    .iter()
                    .zip(ys)
                    .all(|((ka, va), (kb, vb))| values_eq(ka, kb) && values_eq(va, vb))
        }
        _ => a == b,
    }
}

/// The frame type a `Decoded` corresponds to.
enum Expected {
    Request(Request),
    Response(Response),
}

fn expected(d: &Decoded) -> Expected {
    match d {
        Decoded::Request { id, command, args } => Expected::Request(Request {
            id: *id,
            command: command.clone(),
            args: args.iter().map(node_to_value).collect(),
        }),
        Decoded::Response { id, ok, err } => {
            let result = match (ok, err) {
                (Some(node), None) => Ok(node_to_value(node)),
                (None, Some(msg)) => Err(msg.clone()),
                other => panic!("response vector needs exactly one of ok/err: {other:?}"),
            };
            Expected::Response(Response { id: *id, result })
        }
    }
}

impl Expected {
    fn encode(&self) -> Vec<u8> {
        match self {
            Expected::Request(r) => encode_frame(r).unwrap(),
            Expected::Response(r) => encode_frame(r).unwrap(),
        }
    }

    /// Decode one frame from `buf` under `max` and assert it equals `self`
    /// structurally. Returns the bytes consumed.
    fn assert_decodes(&self, buf: &[u8], max: usize, name: &str) -> usize {
        match self {
            Expected::Request(want) => {
                let (got, used): (Request, usize) =
                    decode_frame_with_limit(buf, max).unwrap().unwrap();
                assert_eq!(got.id, want.id, "{name}: id");
                assert_eq!(got.command, want.command, "{name}: command");
                assert_eq!(got.args.len(), want.args.len(), "{name}: arg count");
                for (i, (g, w)) in got.args.iter().zip(&want.args).enumerate() {
                    assert!(values_eq(g, w), "{name}: arg[{i}] {g:?} != {w:?}");
                }
                used
            }
            Expected::Response(want) => {
                let (got, used): (Response, usize) =
                    decode_frame_with_limit(buf, max).unwrap().unwrap();
                assert_eq!(got.id, want.id, "{name}: id");
                match (&got.result, &want.result) {
                    (Ok(g), Ok(w)) => assert!(values_eq(g, w), "{name}: ok {g:?} != {w:?}"),
                    (Err(g), Err(w)) => assert_eq!(g, w, "{name}: err"),
                    (g, w) => panic!("{name}: result arm mismatch: {g:?} vs {w:?}"),
                }
                used
            }
        }
    }
}

#[test]
fn corpus_vectors_hold() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/vectors");
    let mut checked = 0usize;
    for entry in std::fs::read_dir(&dir).expect("conformance/vectors must exist") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let raw = std::fs::read_to_string(&path).unwrap();
        let v: Vector = serde_yaml::from_str(&raw)
            .unwrap_or_else(|e| panic!("{} does not parse: {e}", path.display()));
        let frame = parse_hex(&v.frame_hex);
        let max = v.max_frame_bytes.unwrap_or(DEFAULT_MAX_FRAME_BYTES);

        match v.mode.as_str() {
            "bidirectional" => {
                let want = expected(v.decoded.as_ref().expect("bidirectional needs decoded"));
                // encode(decoded) == frame, byte-exact.
                assert_eq!(want.encode(), frame, "{}: encode mismatch", v.name);
                // decode(frame) == decoded, structurally (floats by bits).
                let used = want.assert_decodes(&frame, max, &v.name);
                assert_eq!(used, frame.len(), "{}: consumed", v.name);
            }
            "decode-only" => {
                let want = expected(v.decoded.as_ref().expect("decode-only needs decoded"));
                let used = want.assert_decodes(&frame, max, &v.name);
                assert_eq!(used, frame.len(), "{}: consumed", v.name);
                // Encoding this form is forbidden: the canonical encoding of
                // the same structure must NOT reproduce the legacy bytes.
                assert_ne!(
                    want.encode(),
                    frame,
                    "{}: legacy form must not be what we emit",
                    v.name
                );
            }
            "stream" => {
                let frames = v.frames.as_ref().expect("stream needs frames");
                let mut offset = 0usize;
                for (i, d) in frames.iter().enumerate() {
                    let used = expected(d).assert_decodes(
                        &frame[offset..],
                        max,
                        &format!("{}[{i}]", v.name),
                    );
                    offset += used;
                }
                assert_eq!(offset, frame.len(), "{}: buffer fully consumed", v.name);
            }
            "incomplete" => {
                let out: Option<(Request, usize)> = decode_frame_with_limit(&frame, max)
                    .unwrap_or_else(|e| panic!("{}: incomplete input must not error: {e}", v.name));
                assert!(out.is_none(), "{}: must ask for more bytes", v.name);
            }
            "reject" => {
                let err = decode_frame_with_limit::<Request>(&frame, max)
                    .expect_err(&format!("{}: must reject", v.name));
                match v.error.as_deref() {
                    Some("frame_too_large") => {
                        assert!(
                            matches!(err, DecodeError::FrameTooLarge { .. }),
                            "{}: expected FrameTooLarge, got {err:?}",
                            v.name
                        );
                    }
                    Some("decode") => {
                        assert!(
                            matches!(err, DecodeError::Rmp(_)),
                            "{}: expected decode error, got {err:?}",
                            v.name
                        );
                    }
                    other => panic!("{}: unknown error class {other:?}", v.name),
                }
            }
            other => panic!("{}: unknown mode {other}", v.name),
        }
        checked += 1;
    }
    assert!(
        checked >= 38,
        "corpus must not silently shrink (found {checked}, floor 38)"
    );
}
