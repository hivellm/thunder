/**
 * Error classification (CLT-050..052) — mirrors the Rust
 * `thunder-client/src/error.rs` test suite exactly.
 */

import { expect, test } from "vitest";

import {
  AuthError,
  ServerError,
  ThunderError,
  classifyServerError,
} from "../src/index";

test("RESP3 auth prefixes map to the auth class (CLT-051)", () => {
  for (const message of [
    "NOAUTH Authentication required.",
    "WRONGPASS invalid username-password pair or user is disabled.",
    "NOPERM this user has no permissions",
    "NOAUTH",
  ]) {
    const error = classifyServerError(message, "resp3_prefixes");
    expect(error, message).toBeInstanceOf(AuthError);
    expect(error.errorClass, message).toBe("auth");
    expect(error.message, message).toBe(message);
    expect(error.code, message).toBeUndefined();
  }
});

test("RESP3 ERR prefix is a generic server error without a code", () => {
  const error = classifyServerError("ERR unknown command", "resp3_prefixes");
  expect(error).toBeInstanceOf(ServerError);
  expect(error.errorClass).toBe("server");
  expect(error.message).toBe("ERR unknown command");
  expect(error.code).toBeUndefined();
});

test("RESP3 prefixes must be word-aligned", () => {
  // "NOAUTHx" is not the NOAUTH prefix.
  const error = classifyServerError("NOAUTHx nope", "resp3_prefixes");
  expect(error).toBeInstanceOf(ServerError);
});

test("bracket code extracts a structured code and keeps the raw message", () => {
  const raw = "[collection_not_found] no such collection: docs";
  const error = classifyServerError(raw, "bracket_code");
  expect(error).toBeInstanceOf(ServerError);
  expect(error.message).toBe(raw);
  expect(error.code).toBe("collection_not_found");
});

test("bracket code still maps auth prefixes to the auth class (CLT-051)", () => {
  const raw = "[unauthorized] NOAUTH token expired";
  const error = classifyServerError(raw, "bracket_code");
  expect(error).toBeInstanceOf(AuthError);
  expect(error.message).toBe(raw);
});

test("the both convention composes bracket and prefixes", () => {
  expect(classifyServerError("[wrongpass] WRONGPASS bad credentials", "both")).toBeInstanceOf(
    AuthError,
  );
  const error = classifyServerError("[index_missing] ERR no such index", "both");
  expect(error).toBeInstanceOf(ServerError);
  expect(error.message).toBe("[index_missing] ERR no such index");
  expect(error.code).toBe("index_missing");
});

test("the none convention never parses", () => {
  const error = classifyServerError("NOAUTH raw passthrough", "none");
  expect(error).toBeInstanceOf(ServerError);
  expect(error.message).toBe("NOAUTH raw passthrough");
  expect(error.code).toBeUndefined();
});

test("malformed bracket prefixes are left alone", () => {
  for (const message of ["[] empty", "[has space] x", "[nospace]tail", "[unclosed"]) {
    const error = classifyServerError(message, "bracket_code");
    expect(error, message).toBeInstanceOf(ServerError);
    expect(error.message, message).toBe(message);
    expect(error.code, message).toBeUndefined();
  }
});

test("every class is a ThunderError with a stable errorClass discriminant (CLT-052)", () => {
  const auth = classifyServerError("NOAUTH x", "resp3_prefixes");
  expect(auth).toBeInstanceOf(ThunderError);
  expect(auth.errorClass).toBe("auth");
  const server = classifyServerError("ERR x", "resp3_prefixes");
  expect(server).toBeInstanceOf(ThunderError);
  expect(server.errorClass).toBe("server");
});
