/** Value factories and accessors (mirrors the Rust `Value` API surface). */

import { expect, test } from "vitest";

import { I64_MAX, I64_MIN, Value } from "../src/index";

test("int accepts numbers within the safe range and bigints within i64", () => {
  expect(Value.int(42)).toEqual({ kind: "int", value: 42n });
  expect(Value.int(-42)).toEqual({ kind: "int", value: -42n });
  expect(Value.int(I64_MIN)).toEqual({ kind: "int", value: I64_MIN });
  expect(Value.int(I64_MAX)).toEqual({ kind: "int", value: I64_MAX });
});

test("int rejects unsafe numbers, non-integers, and out-of-range bigints", () => {
  expect(() => Value.int(1.5)).toThrow(RangeError);
  expect(() => Value.int(Number.NaN)).toThrow(RangeError);
  expect(() => Value.int(Number.MAX_SAFE_INTEGER + 1)).toThrow(RangeError);
  expect(() => Value.int(I64_MAX + 1n)).toThrow(RangeError);
  expect(() => Value.int(I64_MIN - 1n)).toThrow(RangeError);
});

test("accessors extract their variant and reject others", () => {
  expect(Value.asStr(Value.str("x"))).toBe("x");
  expect(Value.asStr(Value.int(1))).toBeUndefined();
  expect(Value.asBool(Value.bool(true))).toBe(true);
  expect(Value.asBool(Value.str("true"))).toBeUndefined();
  expect(Value.asInt(Value.int(7))).toBe(7n);
  expect(Value.asInt(Value.float(7))).toBeUndefined();
  expect(Value.asArray(Value.array([Value.null()]))).toHaveLength(1);
  expect(Value.asMap(Value.map([]))).toEqual([]);
  expect(Value.isNull(Value.null())).toBe(true);
  expect(Value.isNull(Value.str(""))).toBe(false);
  expect(Value.isNull(undefined)).toBe(false);
});

test("asFloat widens Int, asBytes accepts Str as UTF-8 (Rust parity)", () => {
  expect(Value.asFloat(Value.float(1.5))).toBe(1.5);
  expect(Value.asFloat(Value.int(3))).toBe(3);
  expect(Value.asBytes(Value.bytes(Uint8Array.of(1)))).toEqual(Uint8Array.of(1));
  expect(Value.asBytes(Value.str("hi"))).toEqual(Uint8Array.of(0x68, 0x69));
  expect(Value.asBytes(Value.int(1))).toBeUndefined();
});

test("mapGet looks up string keys in order", () => {
  const map = Value.map([
    [Value.str("a"), Value.int(1)],
    [Value.int(2), Value.str("non-string key")],
    [Value.str("b"), Value.null()],
  ]);
  expect(Value.asInt(Value.mapGet(map, "a"))).toBe(1n);
  expect(Value.isNull(Value.mapGet(map, "b") ?? Value.bool(false))).toBe(true);
  expect(Value.mapGet(map, "missing")).toBeUndefined();
  expect(Value.mapGet(Value.str("not a map"), "a")).toBeUndefined();
});
