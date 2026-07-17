/**
 * Protocol configuration (SPEC-002) — the declarative description of how
 * **one application** uses the shared wire. Pure data: the codec never
 * depends on it; the client drives its behavior from it.
 *
 * # Thunder ships one standard and zero product knowledge
 *
 * There are no named configurations here — no per-product constants, no
 * registry. Thunder was born from three products' RPC implementations, but
 * a protocol library that must serve implementations which do not exist yet
 * cannot ship a hardcoded list of the ones that did.
 *
 * Instead: {@link Config.standard} is **the** family standard, and every
 * dimension is a knob. An application that matches the standard writes its
 * identity and nothing else:
 *
 * ```ts
 * import { Config } from "@hivehub/thunder";
 *
 * const config = Config.standard().withScheme("myapp").withPort(9000);
 * ```
 *
 * An application that still diverges says so **in its own repository**,
 * where that knowledge belongs:
 *
 * ```ts
 * // A deployment whose RPC path authenticates via AUTH and has no HELLO
 * // handler, and which ships a push-producing command.
 * const config = Config.standard()
 *   .withScheme("legacy")
 *   .withPort(15501)
 *   .withHandshake("auth_command")
 *   .withHelloStyle("not_used")
 *   .withPush("enabled");
 * ```
 *
 * Convergence is therefore visible and per-application: delete overrides
 * until only `scheme` and `port` remain. Nobody waits on a Thunder release
 * for a row in a registry, and Thunder never carries behavior it does not
 * own.
 *
 * The standard's values are pinned to `conformance/standard.yaml` by a test
 * in every language, so the four implementations can never disagree about
 * what "standard" means — the one guarantee the old per-product registry
 * legitimately provided.
 */

import { DEFAULT_MAX_FRAME_BYTES } from "./wire";

/** Handshake style (PRO-001). */
export type Handshake =
  /** No RPC-layer handshake at all: the connection is usable immediately. */
  | "none"
  /**
   * `HELLO` optional; `AUTH [api_key]` / `[user, pass]` / `[password]`;
   * pre-auth allowlist `PING/HELLO/AUTH/QUIT`.
   *
   * Whether a deployment *enforces* credentials is its own config, not a
   * protocol dialect: a client with no credentials configured simply sends
   * no `AUTH`, which is correct against an open deployment (PRO-001a).
   */
  | "auth_command"
  /**
   * `HELLO` must be the first frame, carrying credentials. **The
   * standard** — see {@link Config.standard}.
   */
  | "hello_mandatory";

/** HELLO payload style (PRO-001). */
export type HelloStyle =
  /** The application has no `HELLO` command. */
  | "not_used"
  /**
   * `HELLO` with **no arguments**; the reply is a metadata Map
   * `{server, version, proto, id, authenticated}`. Credentials travel via
   * `AUTH`, never inside the HELLO.
   */
  | "arg_less"
  /**
   * Map with `version`, `token` | `api_key`, `client_name`; the reply
   * carries `proto` and `capabilities`. **The standard** — the only style
   * that negotiates a version and advertises capabilities, which is what an
   * evolving protocol needs.
   */
  | "map_payload";

/** Server-push policy (PRO-001). */
export type PushPolicy =
  /**
   * `PUSH_ID` reserved: servers refuse it from clients and never emit it.
   * **The standard** — emitting push is a capability an application opts
   * into by shipping a push-producing command.
   */
  | "reserved"
  /** Push frames flow to the client's push hook. */
  | "enabled";

/** Which error-string prefix conventions the client parses (PRO-014). */
export type ErrorConvention =
  /** No prefix parsing. */
  | "none"
  /** `ERR` / `NOAUTH` / `WRONGPASS` / `NOPERM` prefixes. */
  | "resp3_prefixes"
  /** Leading `"[<code>] "` machine-readable code. */
  | "bracket_code"
  /**
   * Both conventions composed. **The standard** — a strict superset, so it
   * parses either grammar and needs no negotiation.
   */
  | "both";

/** Transport-security policy (PRO-001). */
export type TlsPolicy =
  /**
   * Plain TCP. **The standard default** — TLS is an additive capability a
   * deployment turns on, never a dialect.
   */
  | "off"
  /** TLS available behind configuration. */
  | "optional"
  /** Config keys reserved; not wired yet. */
  | "reserved";

/**
 * One application's protocol configuration (PRO-001).
 *
 * Configs are **data, never behavior**: no config may alter wire bytes
 * (PRO-003) — it selects among behaviors Thunder already implements.
 * Build one with {@link Config.standard} and the `with*` overrides, or write
 * a plain object literal; both are supported and neither requires a Thunder
 * release.
 */
export interface Config {
  /**
   * URL scheme the endpoint parser accepts for this application (PRO-012).
   * Identity — Thunder has no default for it.
   */
  readonly scheme: string;
  /**
   * Default RPC port for the scheme (PRO-012). Identity — Thunder has no
   * default for it.
   */
  readonly defaultPort: number;
  /** Handshake style. */
  readonly handshake: Handshake;
  /** HELLO payload style. */
  readonly helloStyle: HelloStyle;
  /** Server-push policy. */
  readonly push: PushPolicy;
  /** Frame cap in bytes (WIRE-020). */
  readonly maxFrameBytes: number;
  /** Per-connection in-flight request bound (CLT-012 / SRV-003). */
  readonly maxInFlight: number;
  /** Error-string conventions the client parses. */
  readonly errorCodes: ErrorConvention;
  /** Transport-security policy. */
  readonly tls: TlsPolicy;
}

/**
 * A {@link Config} plus chainable, immutable overrides — every `with*`
 * returns a **new** frozen config, leaving the receiver untouched.
 *
 * Instances are plain data with the overrides on the prototype: spreading
 * one (`{ ...config, scheme: "x" }`) yields a bare `Config` object, and a
 * bare object literal is accepted anywhere a `Config` is (PRO-003 — a
 * config is data, so nothing forces an application through the builder).
 *
 * The methods carry a `with` prefix because TypeScript cannot give one
 * object a `scheme` property *and* a `scheme()` method, as Rust's builder
 * does.
 */
export class ConfigBuilder implements Config {
  readonly scheme: string;
  readonly defaultPort: number;
  readonly handshake: Handshake;
  readonly helloStyle: HelloStyle;
  readonly push: PushPolicy;
  readonly maxFrameBytes: number;
  readonly maxInFlight: number;
  readonly errorCodes: ErrorConvention;
  readonly tls: TlsPolicy;

  constructor(config: Config) {
    this.scheme = config.scheme;
    this.defaultPort = config.defaultPort;
    this.handshake = config.handshake;
    this.helloStyle = config.helloStyle;
    this.push = config.push;
    this.maxFrameBytes = config.maxFrameBytes;
    this.maxInFlight = config.maxInFlight;
    this.errorCodes = config.errorCodes;
    this.tls = config.tls;
    Object.freeze(this);
  }

  /** Set the URL scheme this application answers on (PRO-012). */
  withScheme(scheme: string): ConfigBuilder {
    return new ConfigBuilder({ ...this, scheme });
  }

  /** Set the default RPC port for the scheme (PRO-012). */
  withPort(defaultPort: number): ConfigBuilder {
    return new ConfigBuilder({ ...this, defaultPort });
  }

  /** Override the handshake style. */
  withHandshake(handshake: Handshake): ConfigBuilder {
    return new ConfigBuilder({ ...this, handshake });
  }

  /** Override the HELLO payload style. */
  withHelloStyle(helloStyle: HelloStyle): ConfigBuilder {
    return new ConfigBuilder({ ...this, helloStyle });
  }

  /** Override the server-push policy. */
  withPush(push: PushPolicy): ConfigBuilder {
    return new ConfigBuilder({ ...this, push });
  }

  /** Override the frame cap (WIRE-020). */
  withMaxFrameBytes(maxFrameBytes: number): ConfigBuilder {
    return new ConfigBuilder({ ...this, maxFrameBytes });
  }

  /** Override the per-connection in-flight bound. */
  withMaxInFlight(maxInFlight: number): ConfigBuilder {
    return new ConfigBuilder({ ...this, maxInFlight });
  }

  /** Override the error-string conventions parsed. */
  withErrorCodes(errorCodes: ErrorConvention): ConfigBuilder {
    return new ConfigBuilder({ ...this, errorCodes });
  }

  /** Override the transport-security policy. */
  withTls(tls: TlsPolicy): ConfigBuilder {
    return new ConfigBuilder({ ...this, tls });
  }
}

export const Config = {
  /**
   * **The** family standard (pinned by `conformance/standard.yaml`).
   *
   * Mandatory `HELLO` map with `proto` negotiation and a capabilities
   * reply; the `[CODE]` error superset; 64 MiB frames; 256 in-flight; push
   * reserved; TLS off.
   *
   * `scheme` is `""` and `defaultPort` is `0` — identity is the
   * application's to supply, and a config that never sets them is only
   * usable with an explicit `host:port` endpoint.
   */
  standard(): ConfigBuilder {
    return new ConfigBuilder({
      scheme: "",
      defaultPort: 0,
      handshake: "hello_mandatory",
      helloStyle: "map_payload",
      push: "reserved",
      maxFrameBytes: DEFAULT_MAX_FRAME_BYTES,
      maxInFlight: 256,
      errorCodes: "both",
      tls: "off",
    });
  },

  /**
   * Lift plain config data into the chainable form — the escape hatch for
   * an application that keeps its config as an object literal (or loads it
   * from its own settings file) and then wants an override.
   */
  from(config: Config): ConfigBuilder {
    return new ConfigBuilder(config);
  },
} as const;
