/**
 * Typed client errors (CLT-050..052).
 *
 * `Response.result` error strings are parsed per the config's
 * `errorCodes` convention (PRO-014) into a {@link ThunderError} carrying
 * the raw message, an optional machine-readable `code` (from a leading
 * `"[code] "` prefix), and a stable error **class**. Application SDKs and
 * user code branch on the class and `code`, never on message text
 * (CLT-052).
 */

import type { ErrorConvention } from "./config";

/** The stable error classes of the client contract (CLT-050). */
export type ErrorClass =
  | "auth"
  | "server"
  | "connection"
  | "timeout"
  | "frame-too-large"
  | "decode";

/**
 * Base class of every Thunder error. The {@link errorClass} discriminant
 * (and the concrete subclasses) are stable public API — matching on them
 * is supported forever (CLT-052).
 */
export class ThunderError extends Error {
  /** Stable error class (CLT-050). */
  readonly errorClass: ErrorClass;
  /**
   * Machine-readable code extracted from a leading `"[code] "` prefix
   * under the `bracket_code` / `both` conventions (PRO-014). Only ever
   * set on the `server` class.
   */
  readonly code: string | undefined;

  constructor(errorClass: ErrorClass, message: string, code?: string) {
    super(message);
    this.name = "ThunderError";
    this.errorClass = errorClass;
    this.code = code;
  }
}

/**
 * Authentication / authorization failure — handshake rejections (CLT-003)
 * and `NOAUTH`/`WRONGPASS`/`NOPERM`-prefixed replies (CLT-051).
 */
export class AuthError extends ThunderError {
  constructor(message: string) {
    super("auth", message);
    this.name = "AuthError";
  }
}

/** The server answered the call with `Result::Err`. */
export class ServerError extends ThunderError {
  constructor(message: string, code?: string) {
    super("server", message, code);
    this.name = "ServerError";
  }
}

/**
 * Transport-level failure: dial, write, or the connection dying while the
 * call was pending (CLT-004/030/031). Also raised for invalid endpoints
 * (CLT-070).
 */
export class ConnectionError extends ThunderError {
  constructor(message: string) {
    super("connection", message);
    this.name = "ConnectionError";
  }
}

/**
 * The per-call (or connect) timeout elapsed (CLT-020). The pending entry
 * was removed; a late response is dropped per CLT-013.
 */
export class TimeoutError extends ThunderError {
  constructor(message = "timed out") {
    super("timeout", message);
    this.name = "TimeoutError";
  }
}

/**
 * A frame's length prefix exceeded the cap; raised before the body buffer
 * is allocated (WIRE-020/021). From a server frame this poisons the
 * connection (CLT-014).
 */
export class FrameTooLargeError extends ThunderError {
  /** Body size the length prefix declared. */
  readonly bodyBytes: number;
  /** The cap that was exceeded. */
  readonly maxBytes: number;

  constructor(bodyBytes: number, maxBytes: number) {
    super(
      "frame-too-large",
      `frame body ${bodyBytes} bytes exceeds limit ${maxBytes} bytes`,
    );
    this.name = "FrameTooLargeError";
    this.bodyBytes = bodyBytes;
    this.maxBytes = maxBytes;
  }
}

/**
 * Malformed frame body (WIRE-023) — or a push frame under a `reserved`
 * config (CLT-060). From a server frame this poisons the connection
 * (CLT-014).
 */
export class DecodeError extends ThunderError {
  constructor(message: string) {
    super("decode", message);
    this.name = "DecodeError";
  }
}

/**
 * Parse a server error string per the config's convention
 * (CLT-050, PRO-014). Mirrors the Rust `ClientError::from_server_message`:
 *
 * - `resp3_prefixes`: `NOAUTH`/`WRONGPASS`/`NOPERM` → {@link AuthError};
 *   everything else (`ERR …` included) → {@link ServerError}.
 * - `bracket_code`: a leading `"[code] "` is extracted into `code`; the
 *   auth prefixes still map to {@link AuthError} regardless of convention
 *   (CLT-051).
 * - `both`: composes the two — bracket code first, then prefixes.
 * - `none`: no parsing; the raw message becomes {@link ServerError}.
 *
 * The error `message` always carries the raw string, verbatim.
 */
export function classifyServerError(
  message: string,
  convention: ErrorConvention,
): ThunderError {
  switch (convention) {
    case "none":
      return new ServerError(message);
    case "resp3_prefixes":
      return startsWithAuthPrefix(message)
        ? new AuthError(message)
        : new ServerError(message);
    case "bracket_code":
    case "both": {
      const { code, rest } = splitBracketCode(message);
      return startsWithAuthPrefix(rest)
        ? new AuthError(message)
        : new ServerError(message, code);
    }
  }
}

const AUTH_PREFIXES = ["NOAUTH", "WRONGPASS", "NOPERM"];

/**
 * True when the message starts with one of the auth prefixes both prefix
 * conventions use for authentication failures (CLT-051). The prefix must
 * be word-aligned (`NOAUTHx` does not count).
 */
function startsWithAuthPrefix(message: string): boolean {
  return AUTH_PREFIXES.some((prefix) => {
    if (!message.startsWith(prefix)) return false;
    const rest = message.slice(prefix.length);
    return rest === "" || rest.startsWith(" ");
  });
}

/**
 * Split a leading `"[code] "` prefix. The code must be non-empty and
 * whitespace-free (machine-readable); anything else
 * leaves the message untouched.
 */
function splitBracketCode(message: string): {
  code: string | undefined;
  rest: string;
} {
  if (message.startsWith("[")) {
    const end = message.indexOf("]");
    if (end > 1) {
      const code = message.slice(1, end);
      if (!/\s/.test(code) && message.startsWith("] ", end)) {
        return { code, rest: message.slice(end + 2) };
      }
    }
  }
  return { code: undefined, rest: message };
}
