/**
 * The 8-variant value model and the `Request`/`Response` frames (WIRE-001/002).
 *
 * `Value` is a discriminated union mirroring the Rust reference enum:
 * `Null | Bool | Int(i64) | Float(f64) | Bytes | Str | Array | Map`.
 * `Int` is a `bigint` (JavaScript numbers cannot carry the full i64 range);
 * the factory accepts plain numbers within the safe-integer range. `Map` is
 * an insertion-ordered pair list because keys may be any value.
 */

/** Smallest i64 value (`i64::MIN`). */
export const I64_MIN = -(2n ** 63n);
/** Largest i64 value (`i64::MAX`). */
export const I64_MAX = 2n ** 63n - 1n;

/** The wire value model (WIRE-002) — byte-compatible across every
 * Thunder language and with the family's pre-Thunder value types. */
export type Value =
  | { kind: "null" }
  | { kind: "bool"; value: boolean }
  | { kind: "int"; value: bigint }
  | { kind: "float"; value: number }
  | { kind: "bytes"; value: Uint8Array }
  | { kind: "str"; value: string }
  | { kind: "array"; value: Value[] }
  | { kind: "map"; value: [Value, Value][] };

const NULL_VALUE: Value = Object.freeze({ kind: "null" });

/** Factories and accessors over {@link Value} (mirrors the Rust `Value` API). */
export const Value = {
  /** SQL NULL / nil. */
  null(): Value {
    return NULL_VALUE;
  },

  bool(value: boolean): Value {
    return { kind: "bool", value };
  },

  /**
   * A 64-bit signed integer. Accepts a `number` within
   * `Number.MIN_SAFE_INTEGER..MAX_SAFE_INTEGER` or a `bigint` within the
   * i64 range; anything else throws a `RangeError`.
   */
  int(value: bigint | number): Value {
    let big: bigint;
    if (typeof value === "number") {
      if (!Number.isSafeInteger(value)) {
        throw new RangeError(
          `Value.int(${value}): number must be a safe integer — pass a bigint for the full i64 range`,
        );
      }
      big = BigInt(value);
    } else {
      big = value;
    }
    if (big < I64_MIN || big > I64_MAX) {
      throw new RangeError(`Value.int(${big}): out of i64 range`);
    }
    return { kind: "int", value: big };
  },

  /** A 64-bit float. Always encoded as MessagePack float64 (WIRE-014). */
  float(value: number): Value {
    return { kind: "float", value };
  },

  /** Raw bytes. Emitted as MessagePack `bin` (WIRE-010). */
  bytes(value: Uint8Array): Value {
    return { kind: "bytes", value };
  },

  str(value: string): Value {
    return { kind: "str", value };
  },

  array(value: Value[]): Value {
    return { kind: "array", value };
  },

  /** Ordered pair list; keys may be any value (WIRE-002). */
  map(value: [Value, Value][]): Value {
    return { kind: "map", value };
  },

  /** Extract the inner string. */
  asStr(value: Value | undefined): string | undefined {
    return value?.kind === "str" ? value.value : undefined;
  },

  /** Extract bytes (also accepts `Str` as UTF-8 bytes). */
  asBytes(value: Value | undefined): Uint8Array | undefined {
    if (value?.kind === "bytes") return value.value;
    if (value?.kind === "str") return new TextEncoder().encode(value.value);
    return undefined;
  },

  /** Extract an integer. */
  asInt(value: Value | undefined): bigint | undefined {
    return value?.kind === "int" ? value.value : undefined;
  },

  /** Extract a float (accepts `Int` widened to a float). */
  asFloat(value: Value | undefined): number | undefined {
    if (value?.kind === "float") return value.value;
    if (value?.kind === "int") return Number(value.value);
    return undefined;
  },

  /** Extract a bool. */
  asBool(value: Value | undefined): boolean | undefined {
    return value?.kind === "bool" ? value.value : undefined;
  },

  /** Extract the array items. */
  asArray(value: Value | undefined): Value[] | undefined {
    return value?.kind === "array" ? value.value : undefined;
  },

  /** Extract the map pairs. */
  asMap(value: Value | undefined): [Value, Value][] | undefined {
    return value?.kind === "map" ? value.value : undefined;
  },

  /** Look up a string key in a `Map` value. */
  mapGet(value: Value | undefined, key: string): Value | undefined {
    if (value?.kind !== "map") return undefined;
    for (const [k, v] of value.value) {
      if (k.kind === "str" && k.value === key) return v;
    }
    return undefined;
  },

  /** True for the `Null` variant. */
  isNull(value: Value | undefined): boolean {
    return value?.kind === "null";
  },
};

/**
 * One RPC request (WIRE-001). `id` is client-chosen (u32) and echoed back;
 * many requests multiplex over one connection. Serialized as an array
 * (WIRE-012); the legacy map shape decodes too (WIRE-013).
 */
export interface Request {
  id: number;
  command: string;
  args: Value[];
}

/**
 * `Response.result`: `{ ok: Value }` or `{ err: string }` — v1 carries no
 * structured error object; conventions are prefix-based and config-driven
 * (WIRE-040).
 */
export type ResponseResult = { ok: Value } | { err: string };

/** One RPC response (WIRE-001). */
export interface Response {
  id: number;
  result: ResponseResult;
}

/** Constructors for {@link Response}. */
export const Response = {
  /** Success response. */
  ok(id: number, value: Value): Response {
    return { id, result: { ok: value } };
  },

  /** Error response with the verbatim error string. */
  err(id: number, message: string): Response {
    return { id, result: { err: message } };
  },
};
