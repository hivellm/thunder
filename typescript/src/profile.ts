/**
 * Protocol profiles (SPEC-002) — the declarative description of how one
 * product uses the shared wire. Pure data: the codec never depends on it;
 * the client drives its behavior from it.
 *
 * The family registry constants below are generated-by-hand from
 * `conformance/profiles/*.yaml` (PRO-010) and pinned to those files by a
 * test — server and SDKs of one product can never disagree. Custom
 * construction stays public (PRO-020): new products never wait for a
 * Thunder release — spread a registry constant or build a fresh object.
 */

import { DEFAULT_MAX_FRAME_BYTES } from "./wire";

/** Handshake style (PRO-001). */
export type Handshake =
  /** No RPC-layer auth (Synap v1 legacy). */
  | "none"
  /** `HELLO` optional; `AUTH [api_key]` or `[user, pass]`; pre-auth
   * allowlist `PING/HELLO/AUTH/QUIT` (Nexus). */
  | "auth_command"
  /** `HELLO` must be the first frame, carrying credentials
   * (Vectorizer / Lexum). */
  | "hello_mandatory";

/** HELLO payload style (PRO-001). */
export type HelloStyle =
  /** No HELLO in the profile (Synap). */
  | "not_used"
  /** Positional `[Int(version)]` (Nexus). */
  | "positional_version"
  /** Map with `version`, `token` | `api_key`, `client_name`; reply
   * carries `capabilities` (Vectorizer / Lexum). */
  | "map_payload";

/** Server-push policy (PRO-001). */
export type PushPolicy =
  /** `PUSH_ID` reserved: servers refuse it from clients and never emit it. */
  | "reserved"
  /** Push frames flow (Synap `SUBSCRIBE`). */
  | "enabled";

/** Which error-string prefix conventions the client parses (PRO-014). */
export type ErrorConvention =
  /** No prefix parsing. */
  | "none"
  /** `ERR` / `NOAUTH` / `WRONGPASS` / `NOPERM` prefixes (Nexus, Synap). */
  | "resp3_prefixes"
  /** Leading `"[<code>] "` machine-readable code (Vectorizer). */
  | "bracket_code"
  /** Both conventions composed (Lexum). */
  | "both";

/** Transport-security policy (PRO-001). */
export type TlsPolicy =
  /** Plain TCP. */
  | "off"
  /** TLS available behind configuration. */
  | "optional"
  /** Config keys reserved; not wired yet. */
  | "reserved";

/**
 * One product's protocol profile (PRO-001). Profiles are data, never
 * behavior: no profile may alter wire bytes (PRO-003).
 */
export interface Profile {
  /** Registry name (`synap`, `nexus`, …) or a custom identifier. */
  readonly name: string;
  /** URL scheme the endpoint parser registers for this profile (PRO-012). */
  readonly scheme: string;
  /** Default RPC port for the scheme (PRO-012). */
  readonly defaultPort: number;
  readonly handshake: Handshake;
  readonly helloStyle: HelloStyle;
  readonly push: PushPolicy;
  /** Frame cap in bytes (WIRE-020). */
  readonly maxFrameBytes: number;
  /** Per-connection in-flight request bound (CLT-012). */
  readonly maxInFlight: number;
  readonly errorCodes: ErrorConvention;
  readonly tls: TlsPolicy;
}

/** Synap — protocol origin. No RPC-layer auth, push enabled, 512 MiB cap
 * (matches `synap-protocol`'s `MAX_FRAME_SIZE`). */
const synap: Profile = Object.freeze({
  name: "synap",
  scheme: "synap",
  defaultPort: 15501,
  handshake: "none",
  helloStyle: "not_used",
  push: "enabled",
  maxFrameBytes: 512 * 1024 * 1024,
  maxInFlight: 256,
  errorCodes: "resp3_prefixes",
  tls: "off",
} satisfies Profile);

/** Nexus — canonical spec author. Optional HELLO + AUTH, 64 MiB cap. */
const nexus: Profile = Object.freeze({
  name: "nexus",
  scheme: "nexus",
  defaultPort: 15475,
  handshake: "auth_command",
  helloStyle: "positional_version",
  push: "reserved",
  maxFrameBytes: DEFAULT_MAX_FRAME_BYTES,
  maxInFlight: 1024,
  errorCodes: "resp3_prefixes",
  tls: "off",
} satisfies Profile);

/** Vectorizer — HELLO-mandatory with credentials, `[code]` prefixes. */
const vectorizer: Profile = Object.freeze({
  name: "vectorizer",
  scheme: "vectorizer",
  defaultPort: 15503,
  handshake: "hello_mandatory",
  helloStyle: "map_payload",
  push: "reserved",
  maxFrameBytes: DEFAULT_MAX_FRAME_BYTES,
  maxInFlight: 256,
  errorCodes: "bracket_code",
  tls: "optional",
} satisfies Profile);

/** Lexum — Vectorizer-style handshake, both error conventions. */
const lexum: Profile = Object.freeze({
  name: "lexum",
  scheme: "lexum",
  defaultPort: 17001,
  handshake: "hello_mandatory",
  helloStyle: "map_payload",
  push: "reserved",
  maxFrameBytes: DEFAULT_MAX_FRAME_BYTES,
  maxInFlight: 256,
  errorCodes: "both",
  tls: "reserved",
} satisfies Profile);

/** The family profile registry (PRO-010/011). */
export const Profiles = {
  synap,
  nexus,
  vectorizer,
  lexum,

  /** Every registered family profile (PRO-010). */
  registry(): readonly Profile[] {
    return [synap, nexus, vectorizer, lexum];
  },
} as const;
