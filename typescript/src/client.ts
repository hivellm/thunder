/**
 * The multiplexed Thunder client (SPEC-003).
 *
 * One {@link Client} owns one TCP connection (CLT-001; pooling is a layer
 * above, CLT-080) and demultiplexes concurrent in-flight calls over it:
 *
 * - ids are monotonically increasing u32s skipping {@link PUSH_ID},
 *   wrapping at the u32 boundary (CLT-010);
 * - the socket's data events feed a {@link FrameReader}; each response is
 *   routed to its caller's promise by id (CLT-010), unknown ids are
 *   dropped and counted (CLT-013), and malformed / oversized frames
 *   poison the connection — every pending call fails with the same typed
 *   error (CLT-014);
 * - writes never interleave (CLT-011): each request is one complete
 *   buffer handed to `socket.write()` on the single-threaded event loop,
 *   and Node preserves write order;
 * - in-flight calls are bounded by the config's `maxInFlight` via a
 *   semaphore — excess calls wait, they are not refused (CLT-012);
 * - per-call timeouts remove the pending entry so a late response falls
 *   under the unknown-id drop (CLT-020); `AbortSignal` cancellation does
 *   the same (CLT-021);
 * - when a call finds the connection dead, the client lazily re-dials and
 *   re-handshakes up to 2 attempts with capped backoff; calls that were
 *   pending when the connection died fail typed and are never replayed
 *   (CLT-030/031);
 * - frames with `id === PUSH_ID` go to the registered push handler under
 *   `push: "enabled"` and poison the connection under `"reserved"`
 *   (CLT-060).
 *
 * The demux architecture mirrors the Rust reference client
 * (`thunder::client`): a reader task plus a pending-call map.
 */

import * as net from "node:net";

import { parseEndpoint } from "./endpoint";
import type { Endpoint } from "./endpoint";
import {
  AuthError,
  ConnectionError,
  DecodeError,
  ServerError,
  ThunderError,
  TimeoutError,
  classifyServerError,
} from "./errors";
import type { Config, PushPolicy } from "./config";
import { FrameReader, PUSH_ID, decodeResponseBody, encodeRequest } from "./wire";
import { Value } from "./value";
import type { Response } from "./value";

/** Reconnect backoff: first re-dial retries after the base, doubling up
 * to the cap (CLT-030 "capped backoff"). */
const BACKOFF_BASE_MS = 50;
const BACKOFF_CAP_MS = 500;

/** Re-dial budget when a call finds the connection dead (CLT-030). */
const RECONNECT_ATTEMPTS = 2;

const DEFAULT_CONNECT_TIMEOUT_MS = 10_000;
const DEFAULT_CALL_TIMEOUT_MS = 30_000;

/**
 * Credentials for the config's handshake (CLT-002). Auth state is
 * per-connection and sticky — there are no per-call credentials (CLT-003).
 */
export type Credentials =
  /** Bearer token (`token` key under `hello_mandatory`). */
  | { type: "token"; token: string }
  /** API key (`api_key` key under `hello_mandatory`, single-arg `AUTH`
   * under `auth_command`). */
  | { type: "apiKey"; apiKey: string }
  /** User + password (`AUTH [user, pass]` under `auth_command`). */
  | { type: "userPass"; user: string; pass: string };

/**
 * Client configuration: connect timeout default **10 s** (CLT-001),
 * per-call timeout default **30 s** (CLT-020), optional credentials and
 * client name for the handshake (CLT-002).
 */
export interface ClientOptions {
  /** TCP connect timeout in milliseconds (CLT-001). Default 10 000. */
  connectTimeoutMs?: number;
  /** Default per-call timeout in milliseconds (CLT-020); override per
   * call via {@link CallOptions.timeoutMs}. Default 30 000. */
  callTimeoutMs?: number;
  /** Handshake credentials, when the config wants them. */
  credentials?: Credentials;
  /** Client identifier sent in the `HELLO` map (`hello_mandatory`). */
  clientName?: string;
}

/** Per-call options. */
export interface CallOptions {
  /** Per-call timeout override in milliseconds (CLT-020). */
  timeoutMs?: number;
  /** Cancels the call, removing the pending entry (CLT-021). The call
   * rejects with the signal's `reason`. */
  signal?: AbortSignal;
}

/** What the handshake learned about this connection (CLT-002). */
export interface HandshakeInfo {
  /** `true` once the server accepted the credentials (`AUTH` succeeded or
   * the `HELLO` reply said so). */
  authenticated: boolean;
  /** Capability names from the `HELLO` reply (`hello_mandatory`). */
  capabilities: string[];
}

/** Outcome of one dispatch attempt on one connection. */
type DispatchOutcome =
  | { kind: "ok"; value: Value }
  /** Final for this call: the frame may have reached the server, or the
   * outcome is a server / timeout / poison error. Never retried. */
  | { kind: "fatal"; error: Error }
  /** The request never reached the wire — safe to resend on a fresh
   * connection (not a replay; CLT-031 concerns frames that were sent). */
  | { kind: "write-failed"; error: ThunderError };

interface PendingEntry {
  onResponse(response: Response): void;
  onError(error: Error): void;
}

function toError(x: unknown): Error {
  return x instanceof Error ? x : new Error(String(x));
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms);
    timer.unref?.();
  });
}

/** In-flight bound (CLT-012): excess calls wait here, never refused. */
class Semaphore {
  #available: number;
  #waiters: { resolve: () => void; reject: (error: Error) => void }[] = [];
  #closed: Error | null = null;

  constructor(permits: number) {
    this.#available = permits;
  }

  acquire(): Promise<void> {
    if (this.#closed) return Promise.reject(this.#closed);
    if (this.#available > 0) {
      this.#available -= 1;
      return Promise.resolve();
    }
    return new Promise((resolve, reject) => {
      this.#waiters.push({ resolve, reject });
    });
  }

  release(): void {
    const next = this.#waiters.shift();
    if (next) next.resolve();
    else this.#available += 1;
  }

  close(error: Error): void {
    this.#closed = error;
    const waiters = this.#waiters;
    this.#waiters = [];
    for (const waiter of waiters) waiter.reject(error);
  }
}

interface ConnHooks {
  pushPolicy: PushPolicy;
  getPushHandler(): ((value: Value) => void) | null;
  onUnknownDrop(): void;
}

/** One live connection: socket + demux state + streaming reader. */
class Conn {
  readonly socket: net.Socket;
  /** id → pending-call demux map (CLT-010). */
  readonly pending = new Map<number, PendingEntry>();
  /** Cleared when the connection is poisoned or closed. */
  alive = true;

  readonly #reader: FrameReader;
  readonly #hooks: ConnHooks;

  constructor(socket: net.Socket, maxFrameBytes: number, hooks: ConnHooks) {
    this.socket = socket;
    this.#reader = new FrameReader({ maxFrameBytes });
    this.#hooks = hooks;
    socket.on("data", (chunk) => {
      this.#onData(chunk);
    });
    socket.on("error", (error) => {
      this.poison(new ConnectionError(`connection lost: ${error.message}`));
    });
    socket.on("close", () => {
      this.poison(new ConnectionError("connection closed"));
    });
  }

  /** The reader path (CLT-010): extract frames under the config's cap,
   * demux by id, route push frames (CLT-060), drop unknown ids (CLT-013),
   * poison on any malformed / oversized frame (CLT-014). */
  #onData(chunk: Uint8Array): void {
    if (!this.alive) return;
    try {
      this.#reader.push(chunk);
      for (;;) {
        const body = this.#reader.nextBody();
        if (body === null) return;
        this.#route(decodeResponseBody(body));
        if (!this.alive) return;
      }
    } catch (e) {
      // FrameTooLargeError / DecodeError keep their class (CLT-050).
      const error =
        e instanceof ThunderError ? e : new DecodeError(toError(e).message);
      this.kill(error);
    }
  }

  #route(response: Response): void {
    if (response.id === PUSH_ID) {
      if (this.#hooks.pushPolicy === "enabled") {
        const handler = this.#hooks.getPushHandler();
        if (handler && "ok" in response.result) {
          try {
            handler(response.result.ok);
          } catch {
            // Push handlers must not take the connection down (CLT-060);
            // offload real work (and error handling) to your own queue.
          }
        }
      } else {
        // Protocol error under a `reserved` config: poison per CLT-014/060.
        this.kill(
          new DecodeError(
            "server sent a push frame but the config reserves PUSH_ID (CLT-060)",
          ),
        );
      }
      return;
    }
    const entry = this.pending.get(response.id);
    if (entry) {
      this.pending.delete(response.id);
      entry.onResponse(response);
    } else {
      // CLT-013: unknown id — count and drop, never fatal.
      this.#hooks.onUnknownDrop();
    }
  }

  /** Poison: mark dead and fail every pending call with the same typed
   * error (CLT-014). Idempotent. */
  poison(error: Error): void {
    this.alive = false;
    const entries = [...this.pending.values()];
    this.pending.clear();
    for (const entry of entries) entry.onError(error);
  }

  /** Tear down: poison and close the socket. */
  kill(error: Error): void {
    this.poison(error);
    this.socket.destroy();
  }
}

/**
 * A multiplexed, config-driven Thunder RPC client (SPEC-003).
 *
 * Every method may be called concurrently; calls multiplex over the one
 * connection and complete in server order, not submission order (CLT-010).
 */
export class Client {
  readonly #config: Config;
  readonly #endpoint: Endpoint;
  readonly #connectTimeoutMs: number;
  readonly #callTimeoutMs: number;
  readonly #credentials: Credentials | undefined;
  readonly #clientName: string | undefined;

  /** Monotonic id allocator, skipping PUSH_ID (CLT-010). */
  #nextId = 1;
  /** In-flight bound sized `config.maxInFlight` (CLT-012). */
  readonly #inFlight: Semaphore;
  /** Current connection; `null` after close. */
  #conn: Conn | null = null;
  /** Serializes re-dial attempts so one caller reconnects at a time. */
  #reconnectLock: Promise<void> = Promise.resolve();
  #closed = false;
  /** Push hook shared with every connection's reader (CLT-060). */
  #pushHandler: ((value: Value) => void) | null = null;
  /** Responses whose id matched no pending call (CLT-013). */
  #unknownDrops = 0;
  #handshakeInfo: HandshakeInfo = { authenticated: false, capabilities: [] };

  private constructor(endpoint: Endpoint, config: Config, options: ClientOptions) {
    this.#endpoint = endpoint;
    this.#config = config;
    this.#connectTimeoutMs = options.connectTimeoutMs ?? DEFAULT_CONNECT_TIMEOUT_MS;
    this.#callTimeoutMs = options.callTimeoutMs ?? DEFAULT_CALL_TIMEOUT_MS;
    this.#credentials = options.credentials;
    this.#clientName = options.clientName;
    this.#inFlight = new Semaphore(config.maxInFlight);
  }

  /**
   * Dial, then run the config's handshake before resolving (CLT-001/002).
   *
   * `endpoint` accepts every form of {@link parseEndpoint} (CLT-070):
   * `scheme://host[:port]` — the scheme being the application's own — or
   * bare `host:port`.
   */
  static async connect(
    endpoint: string,
    config: Config,
    options: ClientOptions = {},
  ): Promise<Client> {
    const client = new Client(parseEndpoint(endpoint, config), config, options);
    client.#conn = await client.#establish();
    return client;
  }

  /**
   * Issue one call (CLT-010/020). Rejects with a typed
   * {@link ThunderError}; server `Err` strings are classified per the
   * config's error convention (CLT-050).
   */
  async call(
    command: string,
    args: Value[] = [],
    options: CallOptions = {},
  ): Promise<Value> {
    const timeoutMs = options.timeoutMs ?? this.#callTimeoutMs;
    const signal = options.signal;
    if (signal?.aborted) throw toError(signal.reason);
    // CLT-012: bounded in-flight — excess calls wait here, never refused.
    await this.#inFlight.acquire();
    try {
      const budget = { left: RECONNECT_ATTEMPTS };
      for (;;) {
        const conn = await this.#liveConn(budget);
        const outcome = await this.#dispatch(conn, command, args, timeoutMs, signal);
        if (outcome.kind === "ok") return outcome.value;
        if (outcome.kind === "fatal") throw outcome.error;
        // The frame never hit the wire: reconnect and resend (CLT-030),
        // unless the re-dial budget is spent.
        if (budget.left === 0) throw outcome.error;
      }
    } finally {
      this.#inFlight.release();
    }
  }

  /**
   * Register the push hook (CLT-060). Frames with `id === PUSH_ID` are
   * routed here under `push: "enabled"` and never matched against pending
   * calls. The handler runs on the socket's data path — keep it fast and
   * offload real work to a queue.
   */
  onPush(handler: (value: Value) => void): void {
    this.#pushHandler = handler;
  }

  /**
   * Explicit, idempotent close (CLT-004): fails all in-flight calls with
   * a typed connection-closed error and destroys the socket.
   */
  close(): Promise<void> {
    if (!this.#closed) {
      this.#closed = true;
      this.#inFlight.close(closedError());
      const conn = this.#conn;
      this.#conn = null;
      if (conn) conn.kill(closedError());
    }
    return Promise.resolve();
  }

  /** `true` once the current connection's handshake authenticated
   * (CLT-003 — auth is sticky per connection). */
  get isAuthenticated(): boolean {
    return this.#handshakeInfo.authenticated;
  }

  /** Capabilities the server advertised in the `HELLO` reply. */
  get capabilities(): string[] {
    return [...this.#handshakeInfo.capabilities];
  }

  /** Snapshot of what the handshake learned (CLT-002). */
  get handshakeInfo(): HandshakeInfo {
    return {
      authenticated: this.#handshakeInfo.authenticated,
      capabilities: [...this.#handshakeInfo.capabilities],
    };
  }

  /** How many responses matched no pending call and were dropped
   * (CLT-013 — client stats, never fatal). */
  get unknownResponseDrops(): number {
    return this.#unknownDrops;
  }

  /** The config this client drives its behavior from. */
  get config(): Config {
    return this.#config;
  }

  // ── internals ──────────────────────────────────────────────────────────

  /** Allocate the next request id, skipping PUSH_ID and wrapping at the
   * u32 boundary (CLT-010). */
  #allocId(): number {
    for (;;) {
      const id = this.#nextId;
      this.#nextId = (this.#nextId + 1) >>> 0;
      if (id !== PUSH_ID) return id;
    }
  }

  async #withReconnectLock<T>(fn: () => Promise<T>): Promise<T> {
    const previous = this.#reconnectLock;
    let release!: () => void;
    this.#reconnectLock = new Promise<void>((resolve) => {
      release = resolve;
    });
    await previous;
    try {
      return await fn();
    } finally {
      release();
    }
  }

  /**
   * Return the current live connection, lazily reconnecting when it is
   * dead or absent: up to `budget.left` re-dial + re-handshake attempts
   * with capped backoff (CLT-030). Never replays in-flight calls — those
   * already failed typed when the connection died (CLT-031).
   */
  async #liveConn(budget: { left: number }): Promise<Conn> {
    if (this.#closed) throw closedError();
    const current = this.#conn;
    if (current?.alive) return current;
    return this.#withReconnectLock(async () => {
      if (this.#closed) throw closedError();
      // Another caller may have reconnected while we waited.
      const existing = this.#conn;
      if (existing?.alive) return existing;
      let lastError: Error = new ConnectionError("connection is dead");
      let backoffMs = BACKOFF_BASE_MS;
      while (budget.left > 0) {
        budget.left -= 1;
        try {
          const conn = await this.#establish();
          this.#conn = conn;
          return conn;
        } catch (e) {
          // An auth rejection is deterministic — retrying cannot fix it.
          if (e instanceof AuthError) throw e;
          lastError = toError(e);
          if (budget.left > 0) {
            await sleep(backoffMs);
            backoffMs = Math.min(backoffMs * 2, BACKOFF_CAP_MS);
          }
        }
      }
      throw lastError;
    });
  }

  /** Dial (with the connect timeout, TCP_NODELAY on — CLT-001), wire the
   * reader, and run the config's handshake (CLT-002). */
  async #establish(): Promise<Conn> {
    const socket = await this.#dial();
    const conn = new Conn(socket, this.#config.maxFrameBytes, {
      pushPolicy: this.#config.push,
      getPushHandler: () => this.#pushHandler,
      onUnknownDrop: () => {
        this.#unknownDrops += 1;
      },
    });
    try {
      this.#handshakeInfo = await this.#handshake(conn);
      return conn;
    } catch (e) {
      // Mirror the Rust drop semantics: a failed handshake tears the
      // connection down before the error propagates.
      conn.kill(new ConnectionError("connection dropped"));
      throw e;
    }
  }

  #dial(): Promise<net.Socket> {
    const { host, port } = this.#endpoint;
    return new Promise((resolve, reject) => {
      const socket = net.connect({ host, port, noDelay: true });
      let settled = false;
      const timer = setTimeout(() => {
        if (settled) return;
        settled = true;
        socket.destroy();
        reject(new TimeoutError(`connect to ${host}:${port} timed out`));
      }, this.#connectTimeoutMs);
      timer.unref?.();
      socket.once("connect", () => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        socket.setNoDelay(true);
        resolve(socket);
      });
      socket.once("error", (error) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        socket.destroy();
        reject(new ConnectionError(`connect to ${host}:${port} failed: ${error.message}`));
      });
    });
  }

  /**
   * Run the configured handshake before user calls proceed (CLT-002):
   * `none` sends nothing; `auth_command` sends the optional arg-less
   * `HELLO` (when the config has one) then `AUTH` when credentials are
   * configured; `hello_mandatory` sends the `HELLO` map as the first frame
   * and parses the reply.
   *
   * Under `auth_command`, no credentials means no `AUTH` frame — which is
   * the correct behavior against a deployment that does not require them.
   * Enforcement is the server's policy, not the config's (PRO-001a).
   */
  async #handshake(conn: Conn): Promise<HandshakeInfo> {
    switch (this.#config.handshake) {
      case "none":
        return { authenticated: false, capabilities: [] };
      case "auth_command": {
        const credentials = this.#credentials;
        if (!credentials) return { authenticated: false, capabilities: [] };
        if (this.#config.helloStyle === "arg_less") {
          // Optional metadata HELLO — takes no arguments; the reply carries
          // {server, version, proto, id, authenticated}. Credentials go in
          // AUTH below.
          await this.#handshakeCall(conn, "HELLO", []);
        }
        const args =
          credentials.type === "token"
            ? [Value.str(credentials.token)]
            : credentials.type === "apiKey"
              ? [Value.str(credentials.apiKey)]
              : [Value.str(credentials.user), Value.str(credentials.pass)];
        await this.#handshakeCall(conn, "AUTH", args);
        return { authenticated: true, capabilities: [] };
      }
      case "hello_mandatory": {
        const pairs: [Value, Value][] = [[Value.str("version"), Value.int(1)]];
        const credentials = this.#credentials;
        if (credentials) {
          if (credentials.type === "token") {
            pairs.push([Value.str("token"), Value.str(credentials.token)]);
          } else if (credentials.type === "apiKey") {
            pairs.push([Value.str("api_key"), Value.str(credentials.apiKey)]);
          } else {
            throw new AuthError(
              "user/password credentials are not supported under the " +
                "hello_mandatory handshake — use a token or api_key (PRO-001)",
            );
          }
        }
        pairs.push([
          Value.str("client_name"),
          Value.str(this.#clientName ?? "thunder-client"),
        ]);
        const reply = await this.#handshakeCall(conn, "HELLO", [Value.map(pairs)]);
        const capabilities = (Value.asArray(Value.mapGet(reply, "capabilities")) ?? [])
          .map((cap) => Value.asStr(cap))
          .filter((cap): cap is string => cap !== undefined);
        return {
          authenticated: Value.asBool(Value.mapGet(reply, "authenticated")) ?? false,
          capabilities,
        };
      }
    }
  }

  /**
   * One handshake round-trip. Server rejections surface as the typed auth
   * class, never a generic error (CLT-003); transport failures keep their
   * own class.
   */
  async #handshakeCall(conn: Conn, command: string, args: Value[]): Promise<Value> {
    const outcome = await this.#dispatch(conn, command, args, this.#callTimeoutMs, undefined);
    if (outcome.kind === "ok") return outcome.value;
    const error = outcome.error;
    if (error instanceof ServerError || error instanceof AuthError) {
      throw new AuthError(error.message);
    }
    throw error;
  }

  /**
   * One request/response attempt on one connection: register the pending
   * entry, write the frame (one buffer, never interleaved — CLT-011),
   * await the demuxed response under the timeout (CLT-020).
   */
  #dispatch(
    conn: Conn,
    command: string,
    args: Value[],
    timeoutMs: number,
    signal: AbortSignal | undefined,
  ): Promise<DispatchOutcome> {
    if (!conn.alive) {
      return Promise.resolve({
        kind: "write-failed",
        error: new ConnectionError("connection is dead"),
      });
    }
    const id = this.#allocId();
    const frame = encodeRequest({ id, command, args });
    return new Promise((resolve) => {
      let done = false;
      const finish = (outcome: DispatchOutcome): void => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        signal?.removeEventListener("abort", onAbort);
        resolve(outcome);
      };
      const onAbort = (): void => {
        conn.pending.delete(id);
        finish({ kind: "fatal", error: toError(signal?.reason) });
      };
      const timer = setTimeout(() => {
        // CLT-020: remove the pending entry on timeout; a late response
        // to this id is dropped per CLT-013.
        conn.pending.delete(id);
        finish({ kind: "fatal", error: new TimeoutError() });
      }, timeoutMs);
      timer.unref?.();
      if (signal) {
        if (signal.aborted) {
          onAbort();
          return;
        }
        // CLT-021: cancellation removes the pending entry.
        signal.addEventListener("abort", onAbort, { once: true });
      }
      conn.pending.set(id, {
        onResponse: (response) => {
          if ("ok" in response.result) {
            finish({ kind: "ok", value: response.result.ok });
          } else {
            finish({
              kind: "fatal",
              error: classifyServerError(response.result.err, this.#config.errorCodes),
            });
          }
        },
        onError: (error) => {
          finish({ kind: "fatal", error });
        },
      });
      try {
        conn.socket.write(frame);
      } catch (e) {
        conn.pending.delete(id);
        const error = new ConnectionError(`write failed: ${toError(e).message}`);
        conn.kill(error);
        finish({ kind: "write-failed", error });
      }
    });
  }
}

function closedError(): ConnectionError {
  return new ConnectionError("client is closed");
}
