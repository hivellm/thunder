//! Reference cross-decode (TST-030): Thunder is pinned to the family's
//! shipping reference implementation (`nexus-protocol`), not to itself.
//! Both directions are asserted over the canonical + value groups:
//! Thunder-encoded frames decode via `nexus_protocol::rpc` into equal
//! structures, and reference-encoded frames decode via `thunder::wire`.
//!
//! ## The one deliberate byte-level asymmetry: `Bytes`
//!
//! Thunder canonically emits `Bytes` as MessagePack **bin** (WIRE-010);
//! the reference still emits the legacy seq-of-ints form (plain `Vec<u8>`
//! without `serde_bytes`). Byte-equality is therefore asserted only for
//! trees without `Bytes`; for `Bytes` trees we assert the stronger interop
//! property instead — each side DECODES the other's frames into equal
//! structures (WIRE-011 tolerance in Thunder; rmp-serde's bin-to-`Vec<u8>`
//! leniency in the reference) — plus an explicit test that the encodings
//! really do differ, so this comment can never rot silently.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use nexus_protocol::rpc as nexus;
use thunder::wire::{decode_frame, encode_frame, Request, Response, Value};

// ── Structural bridges ───────────────────────────────────────────────────────

fn to_nexus(v: &Value) -> nexus::NexusValue {
    match v {
        Value::Null => nexus::NexusValue::Null,
        Value::Bool(b) => nexus::NexusValue::Bool(*b),
        Value::Int(i) => nexus::NexusValue::Int(*i),
        Value::Float(f) => nexus::NexusValue::Float(*f),
        Value::Bytes(b) => nexus::NexusValue::Bytes(b.clone()),
        Value::Str(s) => nexus::NexusValue::Str(s.clone()),
        Value::Array(items) => nexus::NexusValue::Array(items.iter().map(to_nexus).collect()),
        Value::Map(pairs) => nexus::NexusValue::Map(
            pairs
                .iter()
                .map(|(k, val)| (to_nexus(k), to_nexus(val)))
                .collect(),
        ),
    }
}

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

// ── Matrices ─────────────────────────────────────────────────────────────────

fn s(x: &str) -> Value {
    Value::Str(x.to_owned())
}

/// Value trees WITHOUT `Bytes` (and without NaN — NaN gets its own bitwise
/// test): eligible for byte-exact equality between the two encoders.
fn byte_stable_values() -> Vec<Value> {
    vec![
        Value::Null,
        Value::Bool(true),
        Value::Bool(false),
        Value::Int(0),
        Value::Int(-32),
        Value::Int(-33),
        Value::Int(127),
        Value::Int(128),
        Value::Int(255),
        Value::Int(256),
        Value::Int(65535),
        Value::Int(65536),
        Value::Int(i64::MIN),
        Value::Int(i64::MAX),
        Value::Float(1.5),
        Value::Float(-0.0),
        Value::Float(f64::INFINITY),
        Value::Float(f64::NEG_INFINITY),
        Value::Str(String::new()),
        s("héllo wörld"),
        Value::Array(vec![]),
        Value::Map(vec![]),
        Value::Array(vec![Value::Int(1), s("two"), Value::Null]),
        Value::Map(vec![
            (s("k"), Value::Int(1)),
            (Value::Int(2), s("non-string key")),
            (s("nested"), Value::Array(vec![Value::Bool(false)])),
        ]),
    ]
}

/// Value trees WITH `Bytes` (alone and nested): cross-decoded structurally
/// in both directions, byte equality deliberately skipped (see module doc).
fn bytes_values() -> Vec<Value> {
    vec![
        Value::Bytes(vec![]),
        Value::Bytes(vec![1, 2, 3, 255]),
        Value::Array(vec![Value::Bytes(vec![0, 128]), s("mixed")]),
        Value::Map(vec![(s("payload"), Value::Bytes(vec![9, 9, 9]))]),
    ]
}

fn requests(values: Vec<Value>) -> Vec<Request> {
    let mut out = vec![Request {
        id: 1,
        command: "PING".to_owned(),
        args: vec![],
    }];
    out.extend(values.into_iter().enumerate().map(|(i, v)| Request {
        id: i as u32 + 2,
        command: "ECHO".to_owned(),
        args: vec![v],
    }));
    out
}

fn responses(values: Vec<Value>) -> Vec<Response> {
    let mut out: Vec<Response> = values
        .into_iter()
        .enumerate()
        .map(|(i, v)| Response::ok(i as u32 + 2, v))
        .collect();
    out.push(Response::err(9, "ERR unknown command"));
    out.push(Response::err(9, "NOAUTH Authentication required."));
    out.push(Response::err(
        9,
        "[collection_not_found] no such collection: docs",
    ));
    out
}

// ── Thunder encodes → reference decodes ─────────────────────────────────────

#[test]
fn thunder_requests_decode_via_reference() {
    let mut all = byte_stable_values();
    all.extend(bytes_values());
    for req in requests(all) {
        let frame = encode_frame(&req).unwrap();
        let (got, used): (nexus::Request, usize) = nexus::decode_frame(&frame).unwrap().unwrap();
        assert_eq!(used, frame.len(), "{}: consumed", req.command);
        assert_eq!(got.id, req.id);
        assert_eq!(got.command, req.command);
        let want: Vec<nexus::NexusValue> = req.args.iter().map(to_nexus).collect();
        assert_eq!(got.args, want, "reference must decode thunder frame");
    }
}

#[test]
fn thunder_responses_decode_via_reference() {
    let mut all = byte_stable_values();
    all.extend(bytes_values());
    for resp in responses(all) {
        let frame = encode_frame(&resp).unwrap();
        let (got, used): (nexus::Response, usize) = nexus::decode_frame(&frame).unwrap().unwrap();
        assert_eq!(used, frame.len());
        assert_eq!(got.id, resp.id);
        let want = resp.result.as_ref().map(to_nexus).map_err(Clone::clone);
        assert_eq!(got.result, want, "reference must decode thunder frame");
    }
}

// ── Reference encodes → Thunder decodes ─────────────────────────────────────

#[test]
fn reference_requests_decode_via_thunder() {
    let mut all = byte_stable_values();
    all.extend(bytes_values());
    for req in requests(all) {
        let nexus_req = nexus::Request {
            id: req.id,
            command: req.command.clone(),
            args: req.args.iter().map(to_nexus).collect(),
        };
        let frame = nexus::encode_frame(&nexus_req).unwrap();
        let (got, used): (Request, usize) = decode_frame(&frame).unwrap().unwrap();
        assert_eq!(used, frame.len());
        assert_eq!(got.id, req.id);
        assert_eq!(got.command, req.command);
        assert_eq!(got.args.len(), req.args.len());
        for (g, w) in got.args.iter().zip(&req.args) {
            // Bytes arrive in the legacy seq form and must normalize
            // to Value::Bytes (WIRE-011).
            assert!(
                values_eq(g, w),
                "thunder must decode reference frame: {g:?} != {w:?}"
            );
        }
    }
}

#[test]
fn reference_responses_decode_via_thunder() {
    let mut all = byte_stable_values();
    all.extend(bytes_values());
    for resp in responses(all) {
        let nexus_resp = nexus::Response {
            id: resp.id,
            result: resp.result.as_ref().map(to_nexus).map_err(Clone::clone),
        };
        let frame = nexus::encode_frame(&nexus_resp).unwrap();
        let (got, used): (Response, usize) = decode_frame(&frame).unwrap().unwrap();
        assert_eq!(used, frame.len());
        assert_eq!(got.id, resp.id);
        match (&got.result, &resp.result) {
            (Ok(g), Ok(w)) => assert!(values_eq(g, w), "{g:?} != {w:?}"),
            (Err(g), Err(w)) => assert_eq!(g, w),
            (g, w) => panic!("result arm mismatch: {g:?} vs {w:?}"),
        }
    }
}

// ── Byte equality where the encodings must agree exactly ────────────────────

#[test]
fn encodings_are_byte_identical_without_bytes_variant() {
    for req in requests(byte_stable_values()) {
        let nexus_req = nexus::Request {
            id: req.id,
            command: req.command.clone(),
            args: req.args.iter().map(to_nexus).collect(),
        };
        assert_eq!(
            encode_frame(&req).unwrap(),
            nexus::encode_frame(&nexus_req).unwrap(),
            "thunder and the reference must emit identical bytes for {req:?}"
        );
    }
    for resp in responses(byte_stable_values()) {
        let nexus_resp = nexus::Response {
            id: resp.id,
            result: resp.result.as_ref().map(to_nexus).map_err(Clone::clone),
        };
        assert_eq!(
            encode_frame(&resp).unwrap(),
            nexus::encode_frame(&nexus_resp).unwrap(),
            "thunder and the reference must emit identical bytes for {resp:?}"
        );
    }
}

/// Pins the asymmetry the module doc describes: for `Bytes` the encodings
/// genuinely differ (bin vs legacy seq), which is exactly why byte equality
/// is skipped there. If the reference ever adopts bin, this fails and the
/// exemption gets removed.
#[test]
fn bytes_encodings_differ_between_thunder_and_reference() {
    let req = Request {
        id: 1,
        command: "ECHO".to_owned(),
        args: vec![Value::Bytes(vec![1, 2, 3, 255])],
    };
    let nexus_req = nexus::Request {
        id: 1,
        command: "ECHO".to_owned(),
        args: vec![nexus::NexusValue::Bytes(vec![1, 2, 3, 255])],
    };
    assert_ne!(
        encode_frame(&req).unwrap(),
        nexus::encode_frame(&nexus_req).unwrap(),
        "reference adopted bin encoding — remove the Bytes byte-equality exemption"
    );
}

// ── NaN bit pattern crosses both ways ────────────────────────────────────────

#[test]
fn nan_bit_pattern_crosses_both_directions() {
    const PATTERN: u64 = 0x7ff8_0000_0000_0000;

    let frame = encode_frame(&Response::ok(1, Value::Float(f64::from_bits(PATTERN)))).unwrap();
    let (got, _): (nexus::Response, usize) = nexus::decode_frame(&frame).unwrap().unwrap();
    match got.result {
        Ok(nexus::NexusValue::Float(f)) => assert_eq!(f.to_bits(), PATTERN),
        other => panic!("expected Float, got {other:?}"),
    }

    let frame = nexus::encode_frame(&nexus::Response::ok(
        1,
        nexus::NexusValue::Float(f64::from_bits(PATTERN)),
    ))
    .unwrap();
    let (got, _): (Response, usize) = decode_frame(&frame).unwrap().unwrap();
    match got.result {
        Ok(Value::Float(f)) => assert_eq!(f.to_bits(), PATTERN),
        other => panic!("expected Float, got {other:?}"),
    }
}
