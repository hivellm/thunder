/**
 * Wire-codec unit tests mirroring the Rust reference suite
 * (`thunder-wire/src/frame.rs` tests): golden vectors, canonicalization,
 * round trips, framing edges, and the streaming reader.
 */

import { describe, expect, test } from "vitest";

import {
  DEFAULT_MAX_FRAME_BYTES,
  DecodeError,
  FrameReader,
  FrameTooLargeError,
  PUSH_ID,
  Response,
  Value,
  WIRE_VERSION,
  decodeRequest,
  decodeResponse,
  encodeRequest,
  encodeResponse,
} from "../src/index";
import type { Request } from "../src/index";

function hex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join(" ");
}

const bitsView = new DataView(new ArrayBuffer(8));
function floatBits(f: number): bigint {
  bitsView.setFloat64(0, f);
  return bitsView.getBigUint64(0);
}

// ── Golden vectors (family-pinned bytes, corpus canonical group) ───────────

test("PING request matches the family golden vector", () => {
  const request: Request = { id: 1, command: "PING", args: [] };
  const frame = encodeRequest(request);
  expect(hex(frame)).toBe("08 00 00 00 93 01 a4 50 49 4e 47 90");
  const decoded = decodeRequest(frame);
  expect(decoded).not.toBeNull();
  expect(decoded?.message).toEqual(request);
  expect(decoded?.bytesConsumed).toBe(frame.length);
});

test("PONG response matches the nested-Ok golden vector", () => {
  const response = Response.ok(1, Value.str("PONG"));
  const frame = encodeResponse(response);
  // Result<Value, String> nests two one-key maps: {"Ok": {"Str": "PONG"}}.
  expect(hex(frame)).toBe("10 00 00 00 92 01 81 a2 4f 6b 81 a3 53 74 72 a4 50 4f 4e 47");
  const decoded = decodeResponse(frame);
  expect(decoded?.message).toEqual(response);
});

test("Null is a bare string and Int is a single-key map (WIRE-003)", () => {
  const frame = encodeResponse(Response.ok(1, Value.array([Value.null(), Value.int(42)])));
  // ... {"Ok": {"Array": ["Null", {"Int": 42}]}}
  expect(hex(frame)).toContain("a4 4e 75 6c 6c 81 a3 49 6e 74 2a");
});

// ── Bytes canonicalization (WIRE-010/011) ──────────────────────────────────

test("Bytes emit as bin, canonical (WIRE-010)", () => {
  const frame = encodeResponse(Response.ok(1, Value.bytes(Uint8Array.of(1, 2, 3, 255))));
  // {"Bytes": bin8(4)} — c4 04, never the int-array form (94 … cc ff).
  expect(hex(frame)).toContain("81 a5 42 79 74 65 73 c4 04 01 02 03 ff");
});

test("legacy int-array Bytes decode and normalize (WIRE-011)", () => {
  // The seq-of-u8 form every pre-Thunder Rust implementation emits.
  const body = Uint8Array.of(
    0x92, 0x01, 0x81, 0xa2, 0x4f, 0x6b, // [1, {"Ok":
    0x81, 0xa5, 0x42, 0x79, 0x74, 0x65, 0x73, // {"Bytes":
    0x94, 0x01, 0x02, 0x03, 0xcc, 0xff, // [1, 2, 3, 255] as ints
  );
  const frame = new Uint8Array(4 + body.length);
  new DataView(frame.buffer).setUint32(0, body.length, true);
  frame.set(body, 4);
  const decoded = decodeResponse(frame);
  expect(decoded?.message).toEqual(Response.ok(1, Value.bytes(Uint8Array.of(1, 2, 3, 255))));
  // Re-encoding must NOT reproduce the legacy frame (WIRE-011).
  expect(hex(encodeResponse(decodeResponse(frame)!.message))).not.toBe(hex(frame));
});

// ── Round-trip matrix (WIRE-002/014/015) ───────────────────────────────────

test("round trip of every variant", () => {
  const all = Value.array([
    Value.null(),
    Value.bool(true),
    Value.bool(false),
    Value.int(0),
    Value.int(-(2n ** 63n)),
    Value.int(2n ** 63n - 1n),
    Value.int(-32),
    Value.int(127),
    Value.int(255),
    Value.int(65535),
    Value.float(0.0),
    Value.float(-0.0),
    Value.float(Number.POSITIVE_INFINITY),
    Value.float(Number.NEGATIVE_INFINITY),
    Value.bytes(new Uint8Array(0)),
    Value.bytes(Uint8Array.of(0, 1, 2, 255)),
    Value.str(""),
    Value.str("héllo wörld"),
    Value.array([]),
    Value.map([]),
    Value.map([
      [Value.str("k"), Value.int(1)],
      [Value.int(2), Value.str("non-string key")],
    ]),
  ]);
  const frame = encodeResponse(Response.ok(7, all));
  const decoded = decodeResponse(frame);
  expect(decoded?.message).toEqual(Response.ok(7, all));
  expect(decoded?.bytesConsumed).toBe(frame.length);
});

test("NaN bit pattern survives (WIRE-014)", () => {
  const frame = encodeResponse(Response.ok(1, Value.float(Number.NaN)));
  const decoded = decodeResponse(frame);
  const result = decoded?.message.result;
  if (result === undefined || !("ok" in result) || result.ok.kind !== "float") {
    throw new Error("expected a Float Ok result");
  }
  expect(floatBits(result.ok.value)).toBe(floatBits(Number.NaN));
});

test("-0.0 keeps its sign bit through the codec (WIRE-014)", () => {
  const frame = encodeResponse(Response.ok(1, Value.float(-0.0)));
  expect(hex(frame)).toContain("cb 80 00 00 00 00 00 00 00");
});

test("error responses round-trip with both prefix conventions verbatim (WIRE-040)", () => {
  for (const message of [
    "ERR unknown command",
    "NOAUTH Authentication required.",
    "WRONGPASS invalid username-password pair or user is disabled.",
    "[collection_not_found] no such collection: docs",
  ]) {
    const frame = encodeResponse(Response.err(9, message));
    const decoded = decodeResponse(frame);
    expect(decoded?.message.result).toEqual({ err: message });
  }
});

// ── Framing edges (WIRE-020..023) ──────────────────────────────────────────

test("partial header and partial body return null (WIRE-022)", () => {
  const frame = encodeRequest({ id: 1, command: "PING", args: [] });
  for (const cut of [0, 1, 3, 4, frame.length - 1]) {
    expect(decodeRequest(frame.subarray(0, cut)), `cut at ${cut}`).toBeNull();
  }
});

test("two frames in one buffer consume exactly one each (WIRE-022)", () => {
  const a = encodeResponse(Response.ok(1, Value.int(1)));
  const b = encodeResponse(Response.ok(2, Value.int(2)));
  const buf = new Uint8Array(a.length + b.length);
  buf.set(a, 0);
  buf.set(b, a.length);
  const first = decodeResponse(buf);
  expect(first?.message.id).toBe(1);
  expect(first?.bytesConsumed).toBe(a.length);
  const second = decodeResponse(buf.subarray(first?.bytesConsumed ?? 0));
  expect(second?.message.id).toBe(2);
  expect(second?.bytesConsumed).toBe(b.length);
});

test("oversized prefix is rejected before the body arrives (WIRE-020/021)", () => {
  // Only the 4-byte prefix claiming cap+1: the check fires without the
  // body being present at all — allocation cannot have happened.
  const prefix = new Uint8Array(4);
  new DataView(prefix.buffer).setUint32(0, DEFAULT_MAX_FRAME_BYTES + 1, true);
  let caught: unknown;
  try {
    decodeRequest(prefix);
  } catch (e) {
    caught = e;
  }
  expect(caught).toBeInstanceOf(FrameTooLargeError);
  const error = caught as FrameTooLargeError;
  expect(error.bodyBytes).toBe(DEFAULT_MAX_FRAME_BYTES + 1);
  expect(error.maxBytes).toBe(DEFAULT_MAX_FRAME_BYTES);
  expect(error.errorClass).toBe("frame-too-large");
});

test("a custom limit is honored (WIRE-020)", () => {
  const frame = encodeResponse(Response.ok(1, Value.str("x".repeat(100))));
  expect(() => decodeResponse(frame, 8)).toThrow(FrameTooLargeError);
});

test("garbage body is a typed error, not a crash (WIRE-023)", () => {
  const buf = Uint8Array.of(4, 0, 0, 0, 0xc1, 0xc1, 0xc1, 0xc1); // 0xc1 is never valid
  expect(() => decodeRequest(buf)).toThrow(DecodeError);
});

test("zero-length body is a decode error (WIRE-023)", () => {
  expect(() => decodeRequest(Uint8Array.of(0, 0, 0, 0))).toThrow(DecodeError);
});

test("a response body does not decode as a request and vice versa", () => {
  const responseFrame = encodeResponse(Response.ok(1, Value.null()));
  expect(() => decodeRequest(responseFrame)).toThrow(DecodeError);
  const requestFrame = encodeRequest({ id: 1, command: "PING", args: [] });
  expect(() => decodeResponse(requestFrame)).toThrow(DecodeError);
});

test("PUSH_ID is the reserved u32::MAX and round-trips (WIRE-005)", () => {
  expect(PUSH_ID).toBe(4294967295);
  const frame = encodeResponse(Response.ok(PUSH_ID, Value.null()));
  expect(decodeResponse(frame)?.message.id).toBe(PUSH_ID);
});

test("WIRE_VERSION is pinned to 1 (WIRE-004)", () => {
  expect(WIRE_VERSION).toBe(1);
});

// ── FrameReader (WIRE-020/021/022, CLT reader path) ────────────────────────

describe("FrameReader", () => {
  test("reassembles frames from arbitrary chunk boundaries (WIRE-022)", () => {
    const a = encodeResponse(Response.ok(1, Value.str("one")));
    const b = encodeResponse(Response.ok(2, Value.str("two")));
    const joined = new Uint8Array(a.length + b.length);
    joined.set(a, 0);
    joined.set(b, a.length);

    for (const chunkSize of [1, 2, 3, 5, joined.length]) {
      const reader = new FrameReader();
      const bodies: Uint8Array[] = [];
      for (let offset = 0; offset < joined.length; offset += chunkSize) {
        reader.push(joined.subarray(offset, Math.min(offset + chunkSize, joined.length)));
        for (;;) {
          const body = reader.nextBody();
          if (body === null) break;
          bodies.push(body);
        }
      }
      expect(bodies.length, `chunk size ${chunkSize}`).toBe(2);
      expect(reader.bufferedBytes).toBe(0);
    }
  });

  test("throws FrameTooLargeError on the prefix alone (WIRE-020/021)", () => {
    const reader = new FrameReader({ maxFrameBytes: 64 });
    const prefix = new Uint8Array(4);
    new DataView(prefix.buffer).setUint32(0, 65, true);
    reader.push(prefix);
    expect(() => reader.nextBody()).toThrow(FrameTooLargeError);
  });

  test("partial input is never an error (WIRE-022)", () => {
    const reader = new FrameReader();
    reader.push(Uint8Array.of(0x08, 0x00));
    expect(reader.nextBody()).toBeNull();
    reader.push(Uint8Array.of(0x00, 0x00, 0x93));
    expect(reader.nextBody()).toBeNull();
    reader.push(Uint8Array.of(0x01, 0xa4, 0x50, 0x49, 0x4e, 0x47, 0x90));
    const body = reader.nextBody();
    expect(body).not.toBeNull();
    expect(hex(body ?? new Uint8Array(0))).toBe("93 01 a4 50 49 4e 47 90");
  });
});
