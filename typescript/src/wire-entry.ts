/**
 * `@hivehub/thunder/wire` — the wire layer alone, with no Node dependency.
 *
 * The main entry pulls in the client, which statically imports `node:fs`,
 * `node:net` and `node:tls`. A bundler targeting the browser cannot resolve
 * those, so it fails on Thunder even when the consumer touches none of the
 * Node surface — which is what happened to Fluxum's browser SDK, whose
 * transport is `fetch` + `ReadableStream` and which wanted exactly one thing
 * from this package: {@link FrameReader} (GH #10).
 *
 * This entry exposes the codec, the frame reader, the value model, the typed
 * errors and the config types — everything the wire layer is, and nothing that
 * opens a socket, reads a file or negotiates TLS. It is the TypeScript
 * counterpart of the Rust crate's `default-features = false`, which gives the
 * same pure-wire subset for the same reason (WIRE-030: the wire layer is pure
 * in every language).
 *
 * ```ts
 * import { FrameReader, decodeResponseBody } from "@hivehub/thunder/wire";
 *
 * const reader = new FrameReader();
 * reader.push(chunk);
 * for (let body = reader.nextBody(); body; body = reader.nextBody()) {
 *   if (body.length === 0) continue; // keep-alive (WIRE-024)
 *   handle(decodeResponseBody(body));
 * }
 * ```
 *
 * Importing from `@hivehub/thunder` instead gives this plus the Node client.
 */

export { I64_MAX, I64_MIN, Response, Value } from "./value";
export type { Request, ResponseResult } from "./value";

export {
  DEFAULT_MAX_FRAME_BYTES,
  FrameReader,
  PUSH_ID,
  WIRE_VERSION,
  decodeRequest,
  decodeRequestBody,
  decodeResponse,
  decodeResponseBody,
  encodeRequest,
  encodeResponse,
} from "./wire";
export type { DecodedFrame } from "./wire";

export {
  AuthError,
  ConnectionError,
  DecodeError,
  FrameTooLargeError,
  ServerError,
  ThunderError,
  TimeoutError,
  classifyServerError,
} from "./errors";
export type { ErrorClass } from "./errors";

export { Config, ConfigBuilder } from "./config";
export type {
  ErrorConvention,
  Handshake,
  HelloStyle,
  PushPolicy,
  TlsPolicy,
} from "./config";

export { parseEndpoint } from "./endpoint";
export type { Endpoint } from "./endpoint";
