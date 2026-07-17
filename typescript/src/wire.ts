/**
 * Length-prefixed MessagePack frame codec (SPEC-001, wire v1 — frozen).
 *
 * ```text
 * ┌───────────────────┬──────────────────────────┐
 * │  length: u32 (LE) │  body: MessagePack bytes │
 * └───────────────────┴──────────────────────────┘
 *     4 bytes              length bytes
 * ```
 *
 * The body is a `Request` / `Response` in the reference (rmp-serde)
 * externally-tagged encoding over the 8-variant {@link Value} model:
 * unit variants as a bare string (`"Null"`), payload variants as a
 * single-key map (`{"Int": 42}`), `Response.result` as the nested
 * `{"Ok": <value>}` / `{"Err": "<string>"}` (WIRE-003), structs as arrays
 * (WIRE-012).
 *
 * Every scalar, string and binary leaf is encoded/decoded by
 * `@msgpack/msgpack` (the fixed library per WIRE-031); this module only
 * assembles the structural array/map headers the externally-tagged layout
 * requires, because a single object-tree `encode()` call cannot express
 * `Float` leaves that hold integral values (`1.0`, `-0.0`) — JavaScript
 * has one number type and the library would pack them as integers,
 * breaking WIRE-014.
 *
 * Canonicalization (SPEC-001 §2): `Bytes` emit as MessagePack `bin`
 * (WIRE-010) while the legacy int-array form decodes forever (WIRE-011);
 * map-shaped requests decode too (WIRE-013). The cap is validated against
 * the length prefix **before** the body buffer is allocated
 * (WIRE-020/021).
 *
 * This module is pure: no sockets, no timers, no config knowledge
 * (WIRE-030) — it operates on byte buffers only.
 */

import { Decoder, Encoder } from "@msgpack/msgpack";

import { DecodeError, FrameTooLargeError } from "./errors";
import { I64_MAX, I64_MIN, Value } from "./value";
import type { Request, Response } from "./value";

/** Negotiated wire protocol version. v1 is the only version anywhere (WIRE-004). */
export const WIRE_VERSION = 1;

/**
 * Reserved frame id for server-initiated push frames (WIRE-005).
 *
 * Clients must never use it as a request id; servers refuse requests
 * carrying it; client demultiplexers route it to the push hook (CLT-060).
 */
export const PUSH_ID = 0xffff_ffff;

/** Default frame-body cap: 64 MiB, checked against the length prefix
 * before any body allocation (WIRE-020). An application tunes it on its
 * own {@link Config}. */
export const DEFAULT_MAX_FRAME_BYTES = 64 * 1024 * 1024;

const U32_MAX = 0xffff_ffff;

// ── Leaf encoders (the fixed serialization library, WIRE-031) ──────────────

/** Scalars/strings/bytes; bigints as 64-bit forms, numbers compact (WIRE-014). */
const leafEncoder = new Encoder({ useBigInt64: true });
/** Float leaves: always float64, even for integral values (WIRE-014). */
const floatEncoder = new Encoder({ forceIntegerToFloat: true });
/** Body decoder; 64-bit integer forms surface as bigint. */
const bodyDecoder = new Decoder({ useBigInt64: true });

const MIN_SAFE = BigInt(Number.MIN_SAFE_INTEGER);
const MAX_SAFE = BigInt(Number.MAX_SAFE_INTEGER);

const FIXMAP1 = Uint8Array.of(0x81);
const FIXARRAY2 = Uint8Array.of(0x92);
const FIXARRAY3 = Uint8Array.of(0x93);

const packedStr = (s: string): Uint8Array => leafEncoder.encode(s);

const NULL_VARIANT = packedStr("Null");
const TAG_BOOL = packedStr("Bool");
const TAG_INT = packedStr("Int");
const TAG_FLOAT = packedStr("Float");
const TAG_BYTES = packedStr("Bytes");
const TAG_STR = packedStr("Str");
const TAG_ARRAY = packedStr("Array");
const TAG_MAP = packedStr("Map");
const TAG_OK = packedStr("Ok");
const TAG_ERR = packedStr("Err");

/** MessagePack array header in the shortest form. */
function arrayHeader(length: number): Uint8Array {
  if (length < 0x10) return Uint8Array.of(0x90 | length);
  if (length < 0x1_0000) {
    return Uint8Array.of(0xdc, (length >>> 8) & 0xff, length & 0xff);
  }
  return Uint8Array.of(
    0xdd,
    (length >>> 24) & 0xff,
    (length >>> 16) & 0xff,
    (length >>> 8) & 0xff,
    length & 0xff,
  );
}

/** Accumulates one MessagePack body as a chunk list. */
class BodyWriter {
  private readonly chunks: Uint8Array[] = [];
  private size = 0;

  raw(bytes: Uint8Array): void {
    this.chunks.push(bytes);
    this.size += bytes.length;
  }

  /** Encode a scalar / string / bytes leaf via the library. */
  leaf(value: number | bigint | string | boolean | Uint8Array): void {
    this.raw(leafEncoder.encode(value));
  }

  /** Encode a float leaf — always float64 (WIRE-014). */
  float(value: number): void {
    this.raw(floatEncoder.encode(value));
  }

  finish(): Uint8Array {
    const body = new Uint8Array(this.size);
    let offset = 0;
    for (const chunk of this.chunks) {
      body.set(chunk, offset);
      offset += chunk.length;
    }
    return body;
  }
}

function writeInt(writer: BodyWriter, value: bigint): void {
  if (value >= MIN_SAFE && value <= MAX_SAFE) {
    // Compact forms (fixint / u8..u32 / i8..i32, WIRE-014); the library
    // picks the shortest encoding for plain numbers.
    writer.leaf(Number(value));
  } else {
    // 64-bit forms (cf / d3) — the shortest possible beyond 32 bits.
    writer.leaf(value);
  }
}

function writeValue(writer: BodyWriter, value: Value): void {
  switch (value.kind) {
    case "null":
      // Unit variant: the bare string "Null" (WIRE-003).
      writer.raw(NULL_VARIANT);
      return;
    case "bool":
      writer.raw(FIXMAP1);
      writer.raw(TAG_BOOL);
      writer.leaf(value.value);
      return;
    case "int":
      writer.raw(FIXMAP1);
      writer.raw(TAG_INT);
      writeInt(writer, value.value);
      return;
    case "float":
      writer.raw(FIXMAP1);
      writer.raw(TAG_FLOAT);
      writer.float(value.value);
      return;
    case "bytes":
      // Canonical bin form (WIRE-010) — never the legacy int array.
      writer.raw(FIXMAP1);
      writer.raw(TAG_BYTES);
      writer.leaf(value.value);
      return;
    case "str":
      writer.raw(FIXMAP1);
      writer.raw(TAG_STR);
      writer.leaf(value.value);
      return;
    case "array":
      writer.raw(FIXMAP1);
      writer.raw(TAG_ARRAY);
      writer.raw(arrayHeader(value.value.length));
      for (const item of value.value) writeValue(writer, item);
      return;
    case "map":
      // Map is an ordered pair LIST: array of [key, value] arrays (WIRE-002).
      writer.raw(FIXMAP1);
      writer.raw(TAG_MAP);
      writer.raw(arrayHeader(value.value.length));
      for (const [key, val] of value.value) {
        writer.raw(FIXARRAY2);
        writeValue(writer, key);
        writeValue(writer, val);
      }
      return;
  }
}

function writeId(writer: BodyWriter, id: number): void {
  if (!Number.isInteger(id) || id < 0 || id > U32_MAX) {
    throw new RangeError(`frame id ${id} is not a u32`);
  }
  writer.leaf(id);
}

function requestBody(request: Request): Uint8Array {
  // Array-encoded struct [id, command, args] (WIRE-012).
  const writer = new BodyWriter();
  writer.raw(FIXARRAY3);
  writeId(writer, request.id);
  writer.leaf(request.command);
  writer.raw(arrayHeader(request.args.length));
  for (const arg of request.args) writeValue(writer, arg);
  return writer.finish();
}

function responseBody(response: Response): Uint8Array {
  // Array-encoded struct [id, result]; result is the nested
  // {"Ok": value} / {"Err": string} (WIRE-003/012).
  const writer = new BodyWriter();
  writer.raw(FIXARRAY2);
  writeId(writer, response.id);
  writer.raw(FIXMAP1);
  if ("ok" in response.result) {
    writer.raw(TAG_OK);
    writeValue(writer, response.result.ok);
  } else {
    writer.raw(TAG_ERR);
    writer.leaf(response.result.err);
  }
  return writer.finish();
}

function frame(body: Uint8Array): Uint8Array {
  if (body.length > U32_MAX) {
    throw new RangeError(`frame body ${body.length} bytes exceeds the u32 length prefix`);
  }
  const out = new Uint8Array(4 + body.length);
  new DataView(out.buffer).setUint32(0, body.length, true);
  out.set(body, 4);
  return out;
}

/** Encode one complete request frame (`u32 LE length` + body). */
export function encodeRequest(request: Request): Uint8Array {
  return frame(requestBody(request));
}

/** Encode one complete response frame (`u32 LE length` + body). */
export function encodeResponse(response: Response): Uint8Array {
  return frame(responseBody(response));
}

// ── Decode ──────────────────────────────────────────────────────────────────

/** One decoded frame plus the total bytes it consumed (`4 + body`). */
export interface DecodedFrame<T> {
  message: T;
  bytesConsumed: number;
}

function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * Split one frame off `buf`. Returns `null` when the buffer does not yet
 * hold a complete frame (read more and retry — WIRE-022). The cap is
 * validated against the prefix before the body is even looked at
 * (WIRE-020/021), so an oversized prefix rejects even with no body bytes
 * present.
 */
function splitFrame(
  buf: Uint8Array,
  maxFrameBytes: number,
): { body: Uint8Array; total: number } | null {
  if (buf.length < 4) return null;
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  const length = view.getUint32(0, true);
  if (length > maxFrameBytes) {
    throw new FrameTooLargeError(length, maxFrameBytes);
  }
  const total = 4 + length;
  if (buf.length < total) return null;
  return { body: buf.subarray(4, total), total };
}

function decodeBody(body: Uint8Array): unknown {
  try {
    return bodyDecoder.decode(body);
  } catch (e) {
    throw new DecodeError(`malformed MessagePack body: ${errorMessage(e)}`);
  }
}

function isRecord(x: unknown): x is Record<string, unknown> {
  return (
    typeof x === "object" &&
    x !== null &&
    !Array.isArray(x) &&
    !(x instanceof Uint8Array)
  );
}

function toU32(x: unknown, what: string): number {
  if (typeof x === "number" && Number.isInteger(x) && x >= 0 && x <= U32_MAX) {
    return x;
  }
  if (typeof x === "bigint" && x >= 0n && x <= BigInt(U32_MAX)) {
    return Number(x);
  }
  throw new DecodeError(`${what} must be a u32`);
}

function toI64(x: unknown): bigint {
  if (typeof x === "bigint") {
    if (x < I64_MIN || x > I64_MAX) {
      throw new DecodeError(`Int value ${x} is out of i64 range`);
    }
    return x;
  }
  if (typeof x === "number" && Number.isSafeInteger(x)) {
    return BigInt(x);
  }
  throw new DecodeError("Int payload must be an integer");
}

function toF64(x: unknown): number {
  if (typeof x === "number") return x;
  // The reference deserializer widens integer wire values into f64.
  if (typeof x === "bigint") return Number(x);
  throw new DecodeError("Float payload must be a number");
}

function toBytes(x: unknown): Uint8Array {
  if (x instanceof Uint8Array) {
    // Copy: the decoder's output may alias its input buffer.
    return new Uint8Array(x);
  }
  if (Array.isArray(x)) {
    // Legacy tolerance (WIRE-011): Bytes as an array of integers 0–255
    // (pre-v1 encoders) normalizes to the Bytes variant. Emitting it is
    // forbidden.
    const out = new Uint8Array(x.length);
    for (let i = 0; i < x.length; i++) {
      const byte: unknown = x[i];
      if (
        typeof byte !== "number" ||
        !Number.isInteger(byte) ||
        byte < 0 ||
        byte > 255
      ) {
        throw new DecodeError("legacy Bytes array elements must be integers 0–255");
      }
      out[i] = byte;
    }
    return out;
  }
  throw new DecodeError("Bytes payload must be bin data (or the legacy int array)");
}

function toValue(x: unknown): Value {
  if (typeof x === "string") {
    // Unit variant: only "Null" exists (WIRE-003).
    if (x === "Null") return Value.null();
    throw new DecodeError(`unknown unit variant '${x}' (expected "Null")`);
  }
  if (!isRecord(x)) {
    throw new DecodeError("value must be externally tagged (bare string or single-key map)");
  }
  const keys = Object.keys(x);
  const tag = keys[0];
  if (keys.length !== 1 || tag === undefined) {
    throw new DecodeError(
      `value variant must be a single-key map, found ${keys.length} keys`,
    );
  }
  const payload = x[tag];
  switch (tag) {
    case "Bool":
      if (typeof payload !== "boolean") {
        throw new DecodeError("Bool payload must be a boolean");
      }
      return Value.bool(payload);
    case "Int":
      return Value.int(toI64(payload));
    case "Float":
      return Value.float(toF64(payload));
    case "Bytes":
      return Value.bytes(toBytes(payload));
    case "Str":
      if (typeof payload !== "string") {
        throw new DecodeError("Str payload must be a string");
      }
      return Value.str(payload);
    case "Array":
      if (!Array.isArray(payload)) {
        throw new DecodeError("Array payload must be an array");
      }
      return Value.array(payload.map(toValue));
    case "Map": {
      if (!Array.isArray(payload)) {
        throw new DecodeError("Map payload must be an array of pairs");
      }
      const pairs: [Value, Value][] = payload.map((pair: unknown) => {
        if (!Array.isArray(pair) || pair.length !== 2) {
          throw new DecodeError("Map entries must be [key, value] pairs");
        }
        return [toValue(pair[0]), toValue(pair[1])];
      });
      return Value.map(pairs);
    }
    default:
      throw new DecodeError(`unknown value variant '${tag}'`);
  }
}

function toRequest(x: unknown): Request {
  if (Array.isArray(x)) {
    // Canonical array-encoded struct (WIRE-012).
    if (x.length !== 3) {
      throw new DecodeError("request must be [id, command, args]");
    }
    return buildRequest(x[0], x[1], x[2]);
  }
  if (isRecord(x)) {
    // Legacy tolerance (WIRE-013): map-shaped Request
    // {"id": …, "command": …, "args": …} (pre-v1 encoders).
    return buildRequest(x["id"], x["command"], x["args"]);
  }
  throw new DecodeError("request body must be an array-encoded struct (or the legacy map shape)");
}

function buildRequest(id: unknown, command: unknown, args: unknown): Request {
  const idNum = toU32(id, "request id");
  if (typeof command !== "string") {
    throw new DecodeError("request command must be a string");
  }
  if (!Array.isArray(args)) {
    throw new DecodeError("request args must be an array");
  }
  return { id: idNum, command, args: args.map(toValue) };
}

function toResponse(x: unknown): Response {
  if (!Array.isArray(x) || x.length !== 2) {
    throw new DecodeError("response must be [id, result]");
  }
  const id = toU32(x[0], "response id");
  const result: unknown = x[1];
  if (isRecord(result)) {
    const keys = Object.keys(result);
    if (keys.length === 1) {
      if (keys[0] === "Ok") {
        return { id, result: { ok: toValue(result["Ok"]) } };
      }
      if (keys[0] === "Err") {
        const message = result["Err"];
        if (typeof message !== "string") {
          throw new DecodeError("Err payload must be a string");
        }
        return { id, result: { err: message } };
      }
    }
  }
  throw new DecodeError('response result must be {"Ok": value} or {"Err": string}');
}

/** Decode one request frame body (no length prefix). */
export function decodeRequestBody(body: Uint8Array): Request {
  return toRequest(decodeBody(body));
}

/** Decode one response frame body (no length prefix). */
export function decodeResponseBody(body: Uint8Array): Response {
  return toResponse(decodeBody(body));
}

/**
 * Decode one request frame from `buf`.
 *
 * Returns `null` when the buffer does not yet hold a complete frame
 * (WIRE-022). Throws {@link FrameTooLargeError} when the prefix exceeds
 * `maxFrameBytes` — before the body is inspected or allocated
 * (WIRE-020/021) — and {@link DecodeError} for malformed bodies (WIRE-023).
 */
export function decodeRequest(
  buf: Uint8Array,
  maxFrameBytes: number = DEFAULT_MAX_FRAME_BYTES,
): DecodedFrame<Request> | null {
  const split = splitFrame(buf, maxFrameBytes);
  if (split === null) return null;
  return { message: decodeRequestBody(split.body), bytesConsumed: split.total };
}

/** Decode one response frame from `buf` — same contract as {@link decodeRequest}. */
export function decodeResponse(
  buf: Uint8Array,
  maxFrameBytes: number = DEFAULT_MAX_FRAME_BYTES,
): DecodedFrame<Response> | null {
  const split = splitFrame(buf, maxFrameBytes);
  if (split === null) return null;
  return { message: decodeResponseBody(split.body), bytesConsumed: split.total };
}

/**
 * Streaming frame extractor: feed it arbitrary chunks, pull complete
 * frame bodies (WIRE-022 — partial input is never an error, and multiple
 * buffered frames come out one per call).
 *
 * The cap is checked against the length prefix as soon as it is readable
 * (WIRE-020/021): {@link nextBody} throws {@link FrameTooLargeError}
 * without waiting for — or assembling — the oversized body.
 *
 * The reader takes ownership of pushed chunks; bodies it returns are
 * fresh copies, safe to hold.
 */
export class FrameReader {
  #buffer: Uint8Array = new Uint8Array(0);
  readonly #maxFrameBytes: number;

  constructor(options: { maxFrameBytes?: number } = {}) {
    this.#maxFrameBytes = options.maxFrameBytes ?? DEFAULT_MAX_FRAME_BYTES;
  }

  /** Buffered bytes not yet consumed by {@link nextBody}. */
  get bufferedBytes(): number {
    return this.#buffer.length;
  }

  /** Append a chunk received from the transport. */
  push(chunk: Uint8Array): void {
    if (this.#buffer.length === 0) {
      this.#buffer = chunk;
      return;
    }
    const merged = new Uint8Array(this.#buffer.length + chunk.length);
    merged.set(this.#buffer, 0);
    merged.set(chunk, this.#buffer.length);
    this.#buffer = merged;
  }

  /**
   * Extract the next complete frame body, or `null` when more bytes are
   * needed. Throws {@link FrameTooLargeError} on a prefix past the cap.
   */
  nextBody(): Uint8Array | null {
    const split = splitFrame(this.#buffer, this.#maxFrameBytes);
    if (split === null) return null;
    const body = split.body.slice();
    this.#buffer = this.#buffer.subarray(split.total);
    return body;
  }
}
