/**
 * @hivehub/thunder — HiveLLM binary RPC (wire v1, frozen).
 *
 * One frame is `u32 LE length` + MessagePack body over the 8-variant
 * {@link Value} model (SPEC-001). The {@link Client} multiplexes
 * concurrent calls over one TCP connection, driven by the application's
 * own {@link Config} (SPEC-002/003) — Thunder ships one standard and no
 * product knowledge.
 *
 * ```ts
 * import { Client, Config, Value } from "@hivehub/thunder";
 *
 * const config = Config.standard().withScheme("myapp").withPort(9000);
 * const client = await Client.connect("myapp://localhost", config, {
 *   credentials: { type: "apiKey", apiKey: "secret" },
 * });
 * const pong = await client.call("PING");
 * console.log(Value.asStr(pong)); // "PONG"
 * await client.close();
 * ```
 */

export {
  I64_MAX,
  I64_MIN,
  Response,
  Value,
} from "./value";
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

export { Client } from "./client";
export type {
  CallOptions,
  ClientOptions,
  ClientTls,
  Credentials,
  HandshakeInfo,
} from "./client";

export { Pool, PooledConn } from "./pool";
