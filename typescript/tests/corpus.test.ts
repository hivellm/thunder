/**
 * Conformance-corpus loader (TST-020): walks `conformance/vectors/` and
 * asserts every vector per its `mode`. Runs in the default test command —
 * never feature-gated, never skipped (NFR-03).
 *
 * Mode semantics (TST-002, conformance/README.md):
 * - `bidirectional` — `encode(decoded) == frame` byte-exact AND
 *   `decode(frame) == decoded` structurally (floats by bit pattern).
 * - `decode-only`   — decode succeeds and equals `decoded`; the canonical
 *   encoding of `decoded` must NOT reproduce these legacy bytes.
 * - `stream`        — `frames` decode back-to-back, one frame per decode,
 *   consuming the buffer exactly.
 * - `incomplete`    — decoder asks for more bytes (no value, no error).
 * - `reject`        — decode fails with the named error class.
 *
 * `max_frame_bytes` (optional) overrides the 64 MiB default cap so the
 * at-cap / over-cap boundary is testable with real bytes.
 */

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { expect, test } from "vitest";
import { parse } from "yaml";

import {
  DEFAULT_MAX_FRAME_BYTES,
  DecodeError,
  FrameReader,
  FrameTooLargeError,
  Value,
  decodeRequest,
  decodeRequestBody,
  decodeResponse,
  decodeResponseBody,
  encodeRequest,
  encodeResponse,
} from "../src/index";
import type { DecodedFrame, Request, Response } from "../src/index";

const VECTORS_DIR = fileURLToPath(
  new URL("../../conformance/vectors/", import.meta.url),
);

// ── Vector schema (conformance/README.md) ───────────────────────────────────

interface RawNode {
  type: string;
  value?: unknown;
  bits?: string;
}

interface RawDecoded {
  kind: "request" | "response";
  id: bigint | number;
  command?: string;
  args?: RawNode[];
  ok?: RawNode;
  err?: string;
}

interface RawVector {
  name: string;
  group: string;
  mode: string;
  frame_hex: string;
  decoded?: RawDecoded;
  frames?: RawDecoded[];
  error?: string;
  max_frame_bytes?: bigint | number;
  notes?: string;
}

function parseHex(s: string): Uint8Array {
  const parts = s.split(/\s+/).filter((p) => p !== "");
  const out = new Uint8Array(parts.length);
  parts.forEach((part, i) => {
    const byte = Number.parseInt(part, 16);
    if (Number.isNaN(byte) || byte < 0 || byte > 255) {
      throw new Error(`bad hex byte '${part}'`);
    }
    out[i] = byte;
  });
  return out;
}

function hex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join(" ");
}

const bitsView = new DataView(new ArrayBuffer(8));

/** u64 IEEE-754 bit pattern of a float — NaN payloads and the -0.0 sign
 * bit compare correctly where numeric equality cannot (WIRE-014). */
function floatBits(f: number): bigint {
  bitsView.setFloat64(0, f);
  return bitsView.getBigUint64(0);
}

function bitsToFloat(hexBits: string): number {
  bitsView.setBigUint64(0, BigInt(`0x${hexBits}`));
  return bitsView.getFloat64(0);
}

function nodeToValue(n: RawNode): Value {
  switch (n.type) {
    case "null":
      return Value.null();
    case "bool":
      return Value.bool(n.value as boolean);
    case "int":
      return Value.int(n.value as bigint);
    case "float":
      return n.bits !== undefined
        ? Value.float(bitsToFloat(n.bits))
        : Value.float(Number(n.value));
    case "str":
      if (typeof n.value !== "string") throw new Error("str node value must be a string");
      return Value.str(n.value);
    case "bytes":
      if (typeof n.value !== "string") throw new Error("bytes node value must be hex text");
      return Value.bytes(parseHex(n.value));
    case "array":
      return Value.array((n.value as RawNode[]).map(nodeToValue));
    case "map":
      return Value.map(
        (n.value as [RawNode, RawNode][]).map(([k, v]) => [
          nodeToValue(k),
          nodeToValue(v),
        ]),
      );
    default:
      throw new Error(`unknown corpus node type: ${n.type}`);
  }
}

// ── Structural equality (floats by bit pattern — NaN/-0.0 safe) ─────────────

function valuesEq(a: Value, b: Value): boolean {
  switch (a.kind) {
    case "null":
      return b.kind === "null";
    case "bool":
      return b.kind === "bool" && a.value === b.value;
    case "int":
      return b.kind === "int" && a.value === b.value;
    case "float":
      return b.kind === "float" && floatBits(a.value) === floatBits(b.value);
    case "str":
      return b.kind === "str" && a.value === b.value;
    case "bytes":
      return (
        b.kind === "bytes" &&
        a.value.length === b.value.length &&
        a.value.every((byte, i) => byte === b.value[i])
      );
    case "array":
      return (
        b.kind === "array" &&
        a.value.length === b.value.length &&
        a.value.every((item, i) => {
          const other = b.value[i];
          return other !== undefined && valuesEq(item, other);
        })
      );
    case "map":
      return (
        b.kind === "map" &&
        a.value.length === b.value.length &&
        a.value.every(([k, v], i) => {
          const other = b.value[i];
          return other !== undefined && valuesEq(k, other[0]) && valuesEq(v, other[1]);
        })
      );
  }
}

type Expected =
  | { kind: "request"; message: Request }
  | { kind: "response"; message: Response };

function toExpected(d: RawDecoded): Expected {
  const id = Number(d.id);
  if (d.kind === "request") {
    if (typeof d.command !== "string" || !Array.isArray(d.args)) {
      throw new Error("request vector needs command and args");
    }
    return {
      kind: "request",
      message: { id, command: d.command, args: d.args.map(nodeToValue) },
    };
  }
  if (d.ok !== undefined && d.err === undefined) {
    return { kind: "response", message: { id, result: { ok: nodeToValue(d.ok) } } };
  }
  if (d.err !== undefined && d.ok === undefined) {
    return { kind: "response", message: { id, result: { err: d.err } } };
  }
  throw new Error("response vector needs exactly one of ok/err");
}

function encodeExpected(want: Expected): Uint8Array {
  return want.kind === "request"
    ? encodeRequest(want.message)
    : encodeResponse(want.message);
}

/** Decode one frame from `buf` under `max` and assert it equals `want`
 * structurally. Returns the bytes consumed. */
function assertDecodes(
  want: Expected,
  buf: Uint8Array,
  max: number,
  name: string,
): number {
  if (want.kind === "request") {
    const got: DecodedFrame<Request> | null = decodeRequest(buf, max);
    expect(got, `${name}: complete frame must decode`).not.toBeNull();
    if (got === null) throw new Error("unreachable");
    expect(got.message.id, `${name}: id`).toBe(want.message.id);
    expect(got.message.command, `${name}: command`).toBe(want.message.command);
    expect(got.message.args.length, `${name}: arg count`).toBe(want.message.args.length);
    got.message.args.forEach((arg, i) => {
      const wantArg = want.message.args[i];
      expect(
        wantArg !== undefined && valuesEq(arg, wantArg),
        `${name}: arg[${i}] mismatch`,
      ).toBe(true);
    });
    return got.bytesConsumed;
  }
  const got: DecodedFrame<Response> | null = decodeResponse(buf, max);
  expect(got, `${name}: complete frame must decode`).not.toBeNull();
  if (got === null) throw new Error("unreachable");
  expect(got.message.id, `${name}: id`).toBe(want.message.id);
  const gotResult = got.message.result;
  const wantResult = want.message.result;
  if ("ok" in wantResult) {
    expect("ok" in gotResult, `${name}: expected Ok arm`).toBe(true);
    if ("ok" in gotResult) {
      expect(valuesEq(gotResult.ok, wantResult.ok), `${name}: ok value mismatch`).toBe(true);
    }
  } else {
    expect("err" in gotResult, `${name}: expected Err arm`).toBe(true);
    if ("err" in gotResult) {
      expect(gotResult.err, `${name}: err`).toBe(wantResult.err);
    }
  }
  return got.bytesConsumed;
}

// ── The loader ───────────────────────────────────────────────────────────────

const files = readdirSync(VECTORS_DIR)
  .filter((f) => f.endsWith(".yaml"))
  .sort();

const vectors = files.map((file) => {
  const raw = readFileSync(join(VECTORS_DIR, file), "utf8");
  // intAsBigInt: i64 extremes exceed the double-precision safe range.
  return parse(raw, { intAsBigInt: true }) as RawVector;
});

test("corpus does not silently shrink (TST-003 floor)", () => {
  expect(vectors.length).toBeGreaterThanOrEqual(39);
});

for (const vector of vectors) {
  test(`[${vector.mode}] ${vector.name}`, () => {
    const frame = parseHex(vector.frame_hex);
    const max =
      vector.max_frame_bytes !== undefined
        ? Number(vector.max_frame_bytes)
        : DEFAULT_MAX_FRAME_BYTES;

    switch (vector.mode) {
      case "bidirectional": {
        if (vector.decoded === undefined) throw new Error("bidirectional needs decoded");
        const want = toExpected(vector.decoded);
        // encode(decoded) == frame, byte-exact.
        expect(hex(encodeExpected(want)), `${vector.name}: encode`).toBe(hex(frame));
        // decode(frame) == decoded, structurally (floats by bits).
        const used = assertDecodes(want, frame, max, vector.name);
        expect(used, `${vector.name}: consumed`).toBe(frame.length);
        break;
      }
      case "decode-only": {
        if (vector.decoded === undefined) throw new Error("decode-only needs decoded");
        const want = toExpected(vector.decoded);
        const used = assertDecodes(want, frame, max, vector.name);
        expect(used, `${vector.name}: consumed`).toBe(frame.length);
        // Encoding this form is forbidden (WIRE-011/013): the canonical
        // encoding of the same structure must NOT reproduce the legacy bytes.
        expect(hex(encodeExpected(want)), `${vector.name}: legacy form must not be emitted`).not.toBe(
          hex(frame),
        );
        break;
      }
      case "stream": {
        if (vector.frames === undefined) throw new Error("stream needs frames");
        let offset = 0;
        vector.frames.forEach((d, i) => {
          offset += assertDecodes(
            toExpected(d),
            frame.subarray(offset),
            max,
            `${vector.name}[${i}]`,
          );
        });
        expect(offset, `${vector.name}: buffer fully consumed`).toBe(frame.length);

        // The streaming reader agrees: one body per call, buffer drained.
        const reader = new FrameReader({ maxFrameBytes: max });
        reader.push(frame);
        for (const d of vector.frames) {
          const body = reader.nextBody();
          expect(body, `${vector.name}: reader body`).not.toBeNull();
          if (body === null) throw new Error("unreachable");
          const want = toExpected(d);
          if (want.kind === "request") {
            expect(decodeRequestBody(body).id).toBe(want.message.id);
          } else {
            expect(decodeResponseBody(body).id).toBe(want.message.id);
          }
        }
        expect(reader.nextBody(), `${vector.name}: reader drained`).toBeNull();
        expect(reader.bufferedBytes).toBe(0);
        break;
      }
      case "incomplete": {
        // Must ask for more bytes: no value, no error (WIRE-022).
        expect(decodeRequest(frame, max), `${vector.name}: needs more bytes`).toBeNull();
        break;
      }
      case "reject": {
        const attempt = (): unknown => decodeRequest(frame, max);
        switch (vector.error) {
          case "frame_too_large":
            // The cap fires on the prefix alone (WIRE-020/021) — with or
            // without body bytes present, never a "need more" null.
            expect(attempt, `${vector.name}: FrameTooLarge`).toThrow(FrameTooLargeError);
            break;
          case "decode":
            expect(attempt, `${vector.name}: decode error`).toThrow(DecodeError);
            break;
          default:
            throw new Error(`${vector.name}: unknown error class ${String(vector.error)}`);
        }
        break;
      }
      default:
        throw new Error(`${vector.name}: unknown mode ${vector.mode}`);
    }
  });
}
