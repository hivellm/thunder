//! Length-prefixed MessagePack frame codec.
//!
//! ```text
//! ┌───────────────────┬──────────────────────────┐
//! │  length: u32 (LE) │  body: MessagePack bytes │
//! └───────────────────┴──────────────────────────┘
//!     4 bytes              length bytes
//! ```
//!
//! The cap is validated against the length prefix **before** the body
//! buffer is allocated (WIRE-020/021), so a hostile prefix cannot exhaust
//! memory. Decode reports how many bytes one frame consumed, which is also
//! the frame size consumers feed to metrics — nothing ever re-encodes a
//! frame just to measure it (SRV-007).

use serde::{Deserialize, Serialize};

use crate::wire::DEFAULT_MAX_FRAME_BYTES;

/// Errors from the sync decoder.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The length prefix declared a body larger than the caller's cap.
    /// Raised before any body allocation (WIRE-021).
    #[error("frame body {body} bytes exceeds limit {max} bytes")]
    FrameTooLarge { body: usize, max: usize },
    /// Well-formed frame, malformed MessagePack payload (WIRE-023).
    #[error("decode error: {0}")]
    Rmp(#[from] rmp_serde::decode::Error),
}

/// Encode a message into one complete frame (`u32 LE length` + body).
pub fn encode_frame<T: Serialize>(msg: &T) -> Result<Vec<u8>, rmp_serde::encode::Error> {
    let body = rmp_serde::to_vec(msg)?;
    let len = body.len() as u32;
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Decode one frame from a byte slice using [`DEFAULT_MAX_FRAME_BYTES`].
///
/// Returns `Ok(None)` when the buffer does not yet hold a complete frame
/// (read more and retry — WIRE-022). On success returns the value and the
/// total bytes consumed (`4 + body`), which is the frame size for metrics.
pub fn decode_frame<T: for<'de> Deserialize<'de>>(
    buf: &[u8],
) -> Result<Option<(T, usize)>, DecodeError> {
    decode_frame_with_limit(buf, DEFAULT_MAX_FRAME_BYTES)
}

/// Decode one frame, rejecting bodies larger than `max` before the body
/// is even inspected (WIRE-020/021).
pub fn decode_frame_with_limit<T: for<'de> Deserialize<'de>>(
    buf: &[u8],
    max: usize,
) -> Result<Option<(T, usize)>, DecodeError> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len > max {
        return Err(DecodeError::FrameTooLarge { body: len, max });
    }
    let total = 4 + len;
    if buf.len() < total {
        return Ok(None);
    }
    let value = rmp_serde::from_slice(&buf[4..total])?;
    Ok(Some((value, total)))
}

// ── Async helpers (feature = "tokio") ───────────────────────────────────────

#[cfg(feature = "tokio")]
mod tokio_io {
    use std::io;

    use serde::{Deserialize, Serialize};
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

    use crate::wire::value::{Request, Response};
    use crate::wire::DEFAULT_MAX_FRAME_BYTES;

    /// Read one frame; returns the decoded value and the frame size in
    /// bytes (`4 + body` — the metrics input, SRV-007). The cap is checked
    /// between reading the prefix and allocating the body (WIRE-020).
    pub async fn read_frame<T: for<'de> Deserialize<'de>, R: AsyncRead + Unpin>(
        reader: &mut R,
        max: usize,
    ) -> io::Result<(T, usize)> {
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf).await?;
        let len = u32::from_le_bytes(len_buf) as usize;
        if len > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("frame body {len} bytes exceeds limit {max} bytes"),
            ));
        }
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).await?;
        let value = rmp_serde::from_slice(&body)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        Ok((value, 4 + len))
    }

    /// Read one [`Request`] with the default cap.
    pub async fn read_request<R: AsyncRead + Unpin>(
        reader: &mut R,
    ) -> io::Result<(Request, usize)> {
        read_frame(reader, DEFAULT_MAX_FRAME_BYTES).await
    }

    /// Read one [`Request`] with a caller-supplied cap (server hot path).
    pub async fn read_request_with_limit<R: AsyncRead + Unpin>(
        reader: &mut R,
        max: usize,
    ) -> io::Result<(Request, usize)> {
        read_frame(reader, max).await
    }

    /// Read one [`Response`] with the default cap.
    pub async fn read_response<R: AsyncRead + Unpin>(
        reader: &mut R,
    ) -> io::Result<(Response, usize)> {
        read_frame(reader, DEFAULT_MAX_FRAME_BYTES).await
    }

    /// Read one [`Response`] with a caller-supplied cap.
    pub async fn read_response_with_limit<R: AsyncRead + Unpin>(
        reader: &mut R,
        max: usize,
    ) -> io::Result<(Response, usize)> {
        read_frame(reader, max).await
    }

    /// Encode and write one frame; returns the frame size written.
    pub async fn write_frame<T: Serialize, W: AsyncWrite + Unpin>(
        writer: &mut W,
        msg: &T,
    ) -> io::Result<usize> {
        let frame = crate::wire::frame::encode_frame(msg)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        writer.write_all(&frame).await?;
        Ok(frame.len())
    }

    /// Write one [`Request`] frame.
    pub async fn write_request<W: AsyncWrite + Unpin>(
        writer: &mut W,
        req: &Request,
    ) -> io::Result<usize> {
        write_frame(writer, req).await
    }

    /// Write one [`Response`] frame.
    pub async fn write_response<W: AsyncWrite + Unpin>(
        writer: &mut W,
        resp: &Response,
    ) -> io::Result<usize> {
        write_frame(writer, resp).await
    }
}

#[cfg(feature = "tokio")]
pub use tokio_io::{
    read_frame, read_request, read_request_with_limit, read_response, read_response_with_limit,
    write_frame, write_request, write_response,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::wire::value::{Request, Response, Value};
    use crate::wire::PUSH_ID;

    fn hex(bytes: &[u8]) -> String {
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    // ── Golden vectors (family-pinned bytes, corpus canonical group) ───────

    #[test]
    fn ping_request_matches_family_golden_vector() {
        let req = Request {
            id: 1,
            command: "PING".to_owned(),
            args: vec![],
        };
        let frame = encode_frame(&req).unwrap();
        assert_eq!(
            hex(&frame),
            "08 00 00 00 93 01 a4 50 49 4e 47 90",
            "frame must match VECTORIZER_RPC.md §11 / corpus request-ping"
        );
        let (decoded, consumed): (Request, usize) = decode_frame(&frame).unwrap().unwrap();
        assert_eq!(decoded, req);
        assert_eq!(consumed, frame.len());
    }

    #[test]
    fn pong_response_matches_nested_ok_golden_vector() {
        let resp = Response::ok(1, Value::Str("PONG".to_owned()));
        let frame = encode_frame(&resp).unwrap();
        // Result<Value, String> nests two one-key maps: {"Ok": {"Str": "PONG"}}.
        assert_eq!(
            hex(&frame),
            "10 00 00 00 92 01 81 a2 4f 6b 81 a3 53 74 72 a4 50 4f 4e 47"
        );
        let (decoded, _): (Response, usize) = decode_frame(&frame).unwrap().unwrap();
        assert_eq!(decoded, resp);
    }

    #[test]
    fn null_is_bare_string_and_int_is_single_key_map() {
        assert_eq!(
            hex(&rmp_serde::to_vec(&Value::Null).unwrap()),
            "a4 4e 75 6c 6c"
        );
        assert_eq!(
            hex(&rmp_serde::to_vec(&Value::Int(42)).unwrap()),
            "81 a3 49 6e 74 2a"
        );
    }

    // ── Bytes canonicalization (WIRE-010/011, probe T-029) ────────────────

    #[test]
    fn bytes_emit_as_bin_canonical() {
        let encoded = rmp_serde::to_vec(&Value::Bytes(vec![1, 2, 3, 255])).unwrap();
        // {"Bytes": bin8(4)} — c4 04, never the int-array form (94 ... cc ff).
        assert_eq!(hex(&encoded), "81 a5 42 79 74 65 73 c4 04 01 02 03 ff");
    }

    #[test]
    fn bytes_decode_legacy_int_array_form() {
        // The seq-of-u8 form every pre-Thunder Rust implementation emits.
        let legacy: Vec<u8> = vec![
            0x81, 0xa5, 0x42, 0x79, 0x74, 0x65, 0x73, // {"Bytes":
            0x94, 0x01, 0x02, 0x03, 0xcc, 0xff, // [1, 2, 3, 255] as ints
        ];
        let decoded: Value = rmp_serde::from_slice(&legacy).unwrap();
        assert_eq!(decoded, Value::Bytes(vec![1, 2, 3, 255]));
    }

    // ── Request shape tolerance (WIRE-012/013) ────────────────────────────

    #[test]
    fn map_shaped_request_decodes() {
        // Synap Python/Go/Java (≤1.x) encode Request as a named map.
        let req = Request {
            id: 7,
            command: "GET".to_owned(),
            args: vec![Value::Str("key".to_owned())],
        };
        let map_shaped = rmp_serde::to_vec_named(&req).unwrap();
        let array_shaped = rmp_serde::to_vec(&req).unwrap();
        assert_ne!(map_shaped, array_shaped, "shapes must actually differ");
        let decoded: Request = rmp_serde::from_slice(&map_shaped).unwrap();
        assert_eq!(decoded, req);
    }

    // ── Round-trip matrix (donor test suite, WIRE-002/014/015) ────────────

    #[test]
    fn round_trip_all_variants() {
        let all = Value::Array(vec![
            Value::Null,
            Value::Bool(true),
            Value::Bool(false),
            Value::Int(0),
            Value::Int(i64::MIN),
            Value::Int(i64::MAX),
            Value::Int(-32),
            Value::Int(127),
            Value::Int(255),
            Value::Int(65535),
            Value::Float(0.0),
            Value::Float(-0.0),
            Value::Float(f64::INFINITY),
            Value::Float(f64::NEG_INFINITY),
            Value::Bytes(vec![]),
            Value::Bytes(vec![0, 1, 2, 255]),
            Value::Str(String::new()),
            Value::Str("héllo wörld".to_owned()),
            Value::Array(vec![]),
            Value::Map(vec![]),
            Value::Map(vec![
                (Value::Str("k".to_owned()), Value::Int(1)),
                (Value::Int(2), Value::Str("non-string key".to_owned())),
            ]),
        ]);
        let frame = encode_frame(&all).unwrap();
        let (decoded, consumed): (Value, usize) = decode_frame(&frame).unwrap().unwrap();
        assert_eq!(decoded, all);
        assert_eq!(consumed, frame.len());
    }

    #[test]
    fn nan_bit_pattern_survives() {
        let frame = encode_frame(&Value::Float(f64::NAN)).unwrap();
        let (decoded, _): (Value, usize) = decode_frame(&frame).unwrap().unwrap();
        match decoded {
            Value::Float(f) => assert_eq!(f.to_bits(), f64::NAN.to_bits()),
            other => panic!("expected Float, got {other:?}"),
        }
    }

    #[test]
    fn error_response_round_trips_with_prefix_conventions() {
        for msg in [
            "ERR unknown command",
            "NOAUTH Authentication required.",
            "WRONGPASS invalid username-password pair or user is disabled.",
            "[collection_not_found] no such collection: docs",
        ] {
            let resp = Response::err(9, msg);
            let frame = encode_frame(&resp).unwrap();
            let (decoded, _): (Response, usize) = decode_frame(&frame).unwrap().unwrap();
            assert_eq!(decoded.result, Err(msg.to_owned()));
        }
    }

    // ── Framing edges (WIRE-020..023) ─────────────────────────────────────

    #[test]
    fn partial_header_and_partial_body_return_none() {
        let frame = encode_frame(&Request {
            id: 1,
            command: "PING".to_owned(),
            args: vec![],
        })
        .unwrap();
        for cut in [0, 1, 3, 4, frame.len() - 1] {
            let out: Option<(Request, usize)> = decode_frame(&frame[..cut]).unwrap();
            assert!(out.is_none(), "cut at {cut} must ask for more bytes");
        }
    }

    #[test]
    fn two_frames_in_one_buffer_consume_exactly_one_each() {
        let a = encode_frame(&Response::ok(1, Value::Int(1))).unwrap();
        let b = encode_frame(&Response::ok(2, Value::Int(2))).unwrap();
        let mut buf = a.clone();
        buf.extend_from_slice(&b);
        let (first, used): (Response, usize) = decode_frame(&buf).unwrap().unwrap();
        assert_eq!(first.id, 1);
        assert_eq!(used, a.len());
        let (second, used2): (Response, usize) = decode_frame(&buf[used..]).unwrap().unwrap();
        assert_eq!(second.id, 2);
        assert_eq!(used2, b.len());
    }

    #[test]
    fn oversized_prefix_rejected_before_body_arrives() {
        // Only the 4-byte prefix claiming cap+1: the check fires without the
        // body being present at all — allocation cannot have happened.
        let over = (DEFAULT_MAX_FRAME_BYTES + 1) as u32;
        let buf = over.to_le_bytes();
        let err = decode_frame::<Request>(&buf).unwrap_err();
        match err {
            DecodeError::FrameTooLarge { body, max } => {
                assert_eq!(body, DEFAULT_MAX_FRAME_BYTES + 1);
                assert_eq!(max, DEFAULT_MAX_FRAME_BYTES);
            }
            other => panic!("expected FrameTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn custom_limit_is_honored() {
        let frame = encode_frame(&Value::Str("x".repeat(100))).unwrap();
        let err = decode_frame_with_limit::<Value>(&frame, 8).unwrap_err();
        assert!(matches!(err, DecodeError::FrameTooLarge { .. }));
    }

    #[test]
    fn garbage_body_is_a_typed_error_not_a_panic() {
        let mut buf = 4u32.to_le_bytes().to_vec();
        buf.extend_from_slice(&[0xc1, 0xc1, 0xc1, 0xc1]); // 0xc1 is never valid
        let err = decode_frame::<Request>(&buf).unwrap_err();
        assert!(matches!(err, DecodeError::Rmp(_)));
    }

    #[test]
    fn zero_length_body_is_a_decode_error() {
        let buf = 0u32.to_le_bytes();
        let err = decode_frame::<Request>(&buf).unwrap_err();
        assert!(matches!(err, DecodeError::Rmp(_)));
    }

    #[test]
    fn push_id_is_reserved_u32_max() {
        assert_eq!(PUSH_ID, u32::MAX);
    }

    // ── Async path (feature = "tokio") ────────────────────────────────────

    #[cfg(feature = "tokio")]
    #[tokio::test]
    async fn async_write_then_read_reports_frame_size() {
        let req = Request {
            id: 3,
            command: "PING".to_owned(),
            args: vec![],
        };
        let mut buf = Vec::new();
        let written = write_request(&mut buf, &req).await.unwrap();
        assert_eq!(written, buf.len());
        let mut cursor = std::io::Cursor::new(buf);
        let (decoded, size) = read_request(&mut cursor).await.unwrap();
        assert_eq!(decoded, req);
        assert_eq!(size, written);
    }

    #[cfg(feature = "tokio")]
    #[tokio::test]
    async fn async_read_rejects_oversized_prefix_without_reading_body() {
        let over = ((DEFAULT_MAX_FRAME_BYTES + 1) as u32).to_le_bytes();
        let mut cursor = std::io::Cursor::new(over.to_vec());
        let err = read_request(&mut cursor).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
}
