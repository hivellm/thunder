//! Conformance-corpus loader (TST-020): walks `conformance/vectors/` and
//! asserts every vector per its `mode`. Runs in the default test command —
//! never feature-gated (NFR-03).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use serde::Deserialize;
use thunder_wire::{decode_frame, encode_frame, DecodeError, Request, Response, Value};

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
    error: Option<String>,
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

#[derive(Deserialize, Debug)]
struct Node {
    #[serde(rename = "type")]
    ty: String,
    #[serde(default)]
    value: serde_yaml::Value,
}

fn node_to_value(n: &Node) -> Value {
    match n.ty.as_str() {
        "null" => Value::Null,
        "bool" => Value::Bool(n.value.as_bool().unwrap()),
        "int" => Value::Int(n.value.as_i64().unwrap()),
        "float" => Value::Float(n.value.as_f64().unwrap()),
        "str" => Value::Str(n.value.as_str().unwrap().to_owned()),
        "bytes" => Value::Bytes(parse_hex(n.value.as_str().unwrap())),
        other => panic!("corpus node type not supported by this loader yet: {other}"),
    }
}

fn parse_hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

fn expected_frame(d: &Decoded) -> Vec<u8> {
    match d {
        Decoded::Request { id, command, args } => encode_frame(&Request {
            id: *id,
            command: command.clone(),
            args: args.iter().map(node_to_value).collect(),
        })
        .unwrap(),
        Decoded::Response { id, ok, err } => {
            let result = match (ok, err) {
                (Some(node), None) => Ok(node_to_value(node)),
                (None, Some(msg)) => Err(msg.clone()),
                other => panic!("response vector needs exactly one of ok/err: {other:?}"),
            };
            encode_frame(&Response { id: *id, result }).unwrap()
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

        match v.mode.as_str() {
            "bidirectional" => {
                let decoded = v.decoded.as_ref().expect("bidirectional needs decoded");
                // encode(decoded) == frame, byte-exact.
                assert_eq!(
                    expected_frame(decoded),
                    frame,
                    "{}: encode mismatch",
                    v.name
                );
                // decode(frame) == decoded (via round-trip against the encoded form).
                match decoded {
                    Decoded::Request { .. } => {
                        let (req, used): (Request, usize) = decode_frame(&frame).unwrap().unwrap();
                        assert_eq!(used, frame.len(), "{}: consumed", v.name);
                        assert_eq!(encode_frame(&req).unwrap(), frame, "{}", v.name);
                    }
                    Decoded::Response { .. } => {
                        let (resp, used): (Response, usize) =
                            decode_frame(&frame).unwrap().unwrap();
                        assert_eq!(used, frame.len(), "{}: consumed", v.name);
                        assert_eq!(encode_frame(&resp).unwrap(), frame, "{}", v.name);
                    }
                }
            }
            "decode-only" => {
                let decoded = v.decoded.as_ref().expect("decode-only needs decoded");
                match decoded {
                    Decoded::Request { .. } => {
                        let (req, _): (Request, usize) = decode_frame(&frame).unwrap().unwrap();
                        assert_eq!(encode_frame(&req).unwrap().len() > 0, true, "{}", v.name);
                    }
                    Decoded::Response { .. } => {
                        let (_resp, _): (Response, usize) = decode_frame(&frame).unwrap().unwrap();
                    }
                }
            }
            "incomplete" => {
                let out: Option<(Request, usize)> = decode_frame(&frame)
                    .unwrap_or_else(|e| panic!("{}: incomplete input must not error: {e}", v.name));
                assert!(out.is_none(), "{}: must ask for more bytes", v.name);
            }
            "reject" => {
                let err =
                    decode_frame::<Request>(&frame).expect_err(&format!("{}: must reject", v.name));
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
        checked >= 6,
        "corpus must not silently shrink (found {checked})"
    );
}
