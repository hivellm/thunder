/**
 * Behavioral floor tests for the Thunder client (SPEC-003, feeds the
 * CLT-090 suite): in-process `node:net` loopback responders built on the
 * wire codec stand in for `thunder-server` — the client contract is
 * exercised end-to-end over real sockets, mirroring the Rust reference
 * suite (`rust/thunder/tests/behavior.rs`).
 */

import { afterEach, expect, test } from "vitest";

import {
  AuthError,
  Client,
  ConnectionError,
  DecodeError,
  FrameTooLargeError,
  PUSH_ID,
  ServerError,
  ThunderError,
  TimeoutError,
  Value,
} from "../src/index";
import type { ClientOptions, Config, Request } from "../src/index";

import { MockServer } from "./mock-server";

/**
 * A custom config (PRO-020): no handshake, push reserved, no error
 * parsing — the neutral baseline the behavioral tests mutate. Thunder ships
 * no product configs, so the tests build their own — named for the shape
 * they exercise — exactly as an application does.
 */
function plainConfig(overrides: Partial<Config> = {}): Config {
  return {
    scheme: "test",
    defaultPort: 0,
    handshake: "none",
    helloStyle: "not_used",
    push: "reserved",
    maxFrameBytes: 1024 * 1024,
    maxInFlight: 64,
    errorCodes: "none",
    tls: "off",
    ...overrides,
  };
}

/**
 * A config with the `auth_command` shape and **no** HELLO — the shape a
 * deployment whose RPC path authenticates via `AUTH` uses.
 */
function authCommandConfig(overrides: Partial<Config> = {}): Config {
  return plainConfig({
    handshake: "auth_command",
    helloStyle: "not_used",
    errorCodes: "resp3_prefixes",
    ...overrides,
  });
}

/** The `auth_command` shape plus an optional arg-less HELLO. */
function arglessHelloConfig(overrides: Partial<Config> = {}): Config {
  return plainConfig({
    handshake: "auth_command",
    helloStyle: "arg_less",
    errorCodes: "resp3_prefixes",
    ...overrides,
  });
}

/** The standard `hello_mandatory` + map-payload shape. */
function helloMandatoryConfig(overrides: Partial<Config> = {}): Config {
  return plainConfig({
    handshake: "hello_mandatory",
    helloStyle: "map_payload",
    errorCodes: "bracket_code",
    ...overrides,
  });
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  while (cleanups.length > 0) {
    const cleanup = cleanups.pop();
    if (cleanup) await cleanup();
  }
});

async function startServer(): Promise<MockServer> {
  const server = await MockServer.listen();
  cleanups.push(() => server.close());
  return server;
}

async function connect(
  server: MockServer,
  config: Config,
  options?: ClientOptions,
): Promise<Client> {
  const client = await Client.connect(server.addr, config, options);
  cleanups.push(() => client.close());
  return client;
}

// ── Multiplexing (CLT-010/011) ──────────────────────────────────────────────

test("pipelined calls complete out of order", async () => {
  const server = await startServer();
  const client = await connect(server, plainConfig());
  const conn = await server.nextConn();

  const one = client.call("ONE");
  const two = client.call("TWO");
  // Read BOTH requests before answering, then answer in reverse:
  // completion order follows the server, not submission order.
  const first = await conn.nextRequest();
  const second = await conn.nextRequest();
  expect(first.id, "ids must be distinct (CLT-010)").not.toBe(second.id);
  conn.sendOk(second.id, Value.str(second.command));
  conn.sendOk(first.id, Value.str(first.command));

  expect(Value.asStr(await one)).toBe("ONE");
  expect(Value.asStr(await two)).toBe("TWO");
});

test("five pipelined calls complete in a permuted order", async () => {
  // With N=2 a "permutation" can only be the swap, which the reversed case
  // above already covers — and a client that paired replies by arrival
  // order rather than by id would still pass it. Five calls answered in an
  // order that is neither submission nor its reverse can only be routed
  // correctly by the id table (CLT-010/011).
  const REPLY_ORDER = [2, 0, 4, 1, 3];
  const commands = ["C1", "C2", "C3", "C4", "C5"];

  const server = await startServer();
  const client = await connect(server, plainConfig());
  const conn = await server.nextConn();

  const calls = commands.map((command) => client.call(command));
  const requests: Request[] = [];
  for (let i = 0; i < commands.length; i += 1) {
    requests.push(await conn.nextRequest());
  }
  for (const i of REPLY_ORDER) {
    const request = requests[i];
    if (request === undefined) throw new Error(`no request buffered at index ${i}`);
    conn.sendOk(request.id, Value.str(request.command));
  }

  // Each promise resolves with the value carrying ITS OWN command,
  // whatever order the server chose to answer in.
  const results = await Promise.all(calls);
  results.forEach((result, i) => {
    expect(Value.asStr(result), `call ${commands[i]} resolved with another call's reply`).toBe(
      commands[i],
    );
  });
});

test("the in-flight bound backpressures instead of refusing (CLT-012)", async () => {
  const server = await startServer();
  const client = await connect(server, plainConfig({ maxInFlight: 1 }));
  const conn = await server.nextConn();

  const a = client.call("A");
  const b = client.call("B");
  const first = await conn.nextRequest();
  await sleep(100);
  // With maxInFlight = 1 the second call waits for the first permit —
  // its frame has not even been written yet.
  expect(conn.queuedRequests).toBe(0);
  conn.sendOk(first.id, Value.str(first.command));
  const second = await conn.nextRequest();
  conn.sendOk(second.id, Value.str(second.command));

  expect(Value.asStr(await a)).toBe("A");
  expect(Value.asStr(await b)).toBe("B");
});

test("a stray response id is dropped, never fatal (CLT-013)", async () => {
  const server = await startServer();
  const client = await connect(server, plainConfig());
  const conn = await server.nextConn();

  const call = client.call("GET");
  const request = await conn.nextRequest();
  // A response nobody asked for, then the real one.
  conn.sendOk(9_999, Value.null());
  conn.sendOk(request.id, Value.str("real"));

  expect(Value.asStr(await call)).toBe("real");
  expect(client.unknownResponseDrops).toBe(1);
});

// ── Handshakes (CLT-002/003) ────────────────────────────────────────────────

test("the none handshake sends nothing before user calls", async () => {
  const server = await startServer();
  // `plainConfig()` is the genuine `none` case (PRO-020).
  const client = await connect(
    server,
    plainConfig({ push: "enabled", errorCodes: "resp3_prefixes" }),
  );
  expect(client.isAuthenticated).toBe(false);

  const pong = client.call("PING");
  const conn = await server.nextConn();
  // The very first frame must be the user's command — no HELLO, no AUTH.
  const request = await conn.nextRequest();
  expect(request.command).toBe("PING");
  conn.sendOk(request.id, Value.str("PONG"));
  expect(Value.asStr(await pong)).toBe("PONG");
});

/**
 * The client half of the shape/policy split (PRO-001a): under the
 * `auth_command` shape with no credentials configured the client sends no
 * `AUTH` at all — exactly right against an open deployment, whose
 * enforcement toggle is the server's policy, not a dialect. It must also
 * never send `HELLO` (`helloStyle: "not_used"`).
 */
test("the auth_command shape without credentials sends nothing", async () => {
  const server = await startServer();
  const client = await connect(server, authCommandConfig());
  expect(client.isAuthenticated).toBe(false);

  const pong = client.call("PING");
  const conn = await server.nextConn();
  const request = await conn.nextRequest();
  expect(request.command, "no AUTH/HELLO frame without credentials").toBe("PING");
  conn.sendOk(request.id, Value.str("PONG"));
  expect(Value.asStr(await pong)).toBe("PONG");
});

/**
 * BN-023 regression: the `auth_command` + `not_used` shape must be able to
 * authenticate — a deployment whose RPC path has an `AUTH` handler and no
 * `HELLO` handler is exactly this shape. Read as `handshake: "none"`, a
 * credentialed client would send **nothing** and could never reach an
 * auth-requiring deployment.
 */
test("the auth_command shape sends AUTH and never HELLO", async () => {
  const server = await startServer();
  const clientPromise = Client.connect(server.addr, authCommandConfig(), {
    credentials: { type: "userPass", user: "root", pass: "hunter2" },
  });
  void clientPromise.catch(() => undefined);

  const conn = await server.nextConn();
  // First frame must be AUTH — this shape has no HELLO command at all.
  const auth = await conn.nextRequest();
  expect(auth.command, "first frame must be AUTH, not HELLO").toBe("AUTH");
  expect(auth.args, "the AUTH <user> <password> form").toEqual([
    Value.str("root"),
    Value.str("hunter2"),
  ]);
  conn.sendOk(auth.id, Value.str("OK"));

  const client = await clientPromise;
  cleanups.push(() => client.close());
  expect(client.isAuthenticated).toBe(true);

  const pong = client.call("PING");
  const ping = await conn.nextRequest();
  expect(ping.command).toBe("PING");
  conn.sendOk(ping.id, Value.str("PONG"));
  expect(Value.asStr(await pong)).toBe("PONG");
});

test("auth_command sends HELLO then AUTH with an api key", async () => {
  const server = await startServer();
  const clientPromise = Client.connect(server.addr, arglessHelloConfig(), {
    credentials: { type: "apiKey", apiKey: "k-123" },
  });
  void clientPromise.catch(() => undefined);

  const conn = await server.nextConn();
  const hello = await conn.nextRequest();
  expect(hello.command).toBe("HELLO");
  expect(
    hello.args,
    "the arg-less HELLO takes no arguments — a positional [Int(1)] would be " +
      "the RESP3 HELLO, a different surface (BN-023 errata)",
  ).toEqual([]);
  conn.sendOk(hello.id, Value.null());
  const auth = await conn.nextRequest();
  expect(auth.command).toBe("AUTH");
  expect(auth.args).toEqual([Value.str("k-123")]);
  conn.sendOk(auth.id, Value.str("OK"));

  const client = await clientPromise;
  cleanups.push(() => client.close());
  expect(client.isAuthenticated).toBe(true);

  const pong = client.call("PING");
  const ping = await conn.nextRequest();
  expect(ping.command).toBe("PING");
  conn.sendOk(ping.id, Value.str("PONG"));
  expect(Value.asStr(await pong)).toBe("PONG");
});

test("auth_command sends user + pass", async () => {
  const server = await startServer();
  const clientPromise = Client.connect(server.addr, arglessHelloConfig(), {
    credentials: { type: "userPass", user: "admin", pass: "hunter2" },
  });
  void clientPromise.catch(() => undefined);

  const conn = await server.nextConn();
  const hello = await conn.nextRequest();
  expect(hello.command).toBe("HELLO");
  conn.sendOk(hello.id, Value.null());
  const auth = await conn.nextRequest();
  expect(auth.command).toBe("AUTH");
  expect(auth.args).toEqual([Value.str("admin"), Value.str("hunter2")]);
  conn.sendOk(auth.id, Value.str("OK"));

  const client = await clientPromise;
  cleanups.push(() => client.close());
  expect(client.isAuthenticated).toBe(true);
});

test("auth_command without credentials sends nothing", async () => {
  const server = await startServer();
  const client = await connect(server, arglessHelloConfig());

  const pong = client.call("PING");
  const conn = await server.nextConn();
  const request = await conn.nextRequest();
  expect(request.command, "no HELLO/AUTH without credentials").toBe("PING");
  conn.sendOk(request.id, Value.str("PONG"));
  await pong;
  expect(client.isAuthenticated).toBe(false);
});

test("hello_mandatory sends the HELLO map first and exposes capabilities", async () => {
  const server = await startServer();
  const clientPromise = Client.connect(server.addr, helloMandatoryConfig(), {
    credentials: { type: "token", token: "tok-1" },
    clientName: "itest",
  });
  void clientPromise.catch(() => undefined);

  const conn = await server.nextConn();
  const hello = await conn.nextRequest();
  expect(hello.command, "HELLO must be the first frame").toBe("HELLO");
  const map = hello.args[0];
  expect(Value.asInt(Value.mapGet(map, "version"))).toBe(1n);
  expect(
    Value.asStr(Value.mapGet(map, "token")),
    "token credential goes in the HELLO map",
  ).toBe("tok-1");
  expect(Value.asStr(Value.mapGet(map, "client_name"))).toBe("itest");
  conn.sendOk(
    hello.id,
    Value.map([
      [Value.str("authenticated"), Value.bool(true)],
      [Value.str("capabilities"), Value.array([Value.str("search"), Value.str("insert")])],
    ]),
  );

  const client = await clientPromise;
  cleanups.push(() => client.close());
  expect(client.isAuthenticated).toBe(true);
  expect(client.capabilities).toEqual(["search", "insert"]);
});

test("a handshake rejection is a typed auth error (CLT-003)", async () => {
  const server = await startServer();
  const clientPromise = Client.connect(server.addr, helloMandatoryConfig(), {
    credentials: { type: "apiKey", apiKey: "wrong" },
  });
  void clientPromise.catch(() => undefined);

  const conn = await server.nextConn();
  const hello = await conn.nextRequest();
  conn.sendErr(hello.id, "[unauthorized] invalid api key");

  let caught: unknown;
  try {
    await clientPromise;
  } catch (e) {
    caught = e;
  }
  // CLT-003: an auth failure is the auth class, not a generic error.
  expect(caught).toBeInstanceOf(AuthError);
  expect((caught as AuthError).message).toContain("unauthorized");
});

// ── Timeouts and cancellation (CLT-020/021) ─────────────────────────────────

/**
 * TEST-NET-1 (RFC 5737) — reserved for documentation, routable nowhere. A
 * SYN to it is dropped rather than refused, so the dial hangs and the
 * connect timeout is what ends it. A closed port on localhost would not do:
 * that is refused instantly, which is the ConnectionError class, not this
 * one.
 */
const BLACKHOLE_ADDR = "192.0.2.1:9";

test("the connect timeout fires as a typed timeout (CLT-001)", async () => {
  const started = Date.now();
  const connecting = Client.connect(BLACKHOLE_ADDR, plainConfig(), {
    connectTimeoutMs: 150,
  });
  // A dial that never completes is the timeout class — not a connection
  // error, and not a hang.
  await expect(connecting).rejects.toBeInstanceOf(TimeoutError);
  expect(
    Date.now() - started,
    "the dial must be given the full connect timeout before failing",
  ).toBeGreaterThanOrEqual(150);
});

test("the per-call timeout fires and the late response is dropped", async () => {
  const server = await startServer();
  const client = await connect(server, plainConfig());
  const conn = await server.nextConn();

  const slowCall = client.call("SLOW", [], { timeoutMs: 100 });
  void slowCall.catch(() => undefined);
  const slow = await conn.nextRequest();

  await expect(slowCall).rejects.toBeInstanceOf(TimeoutError);

  // Answer only after the next request proves the timeout fired
  // client-side; deliver the late response first.
  const freshCall = client.call("NEXT");
  const next = await conn.nextRequest();
  conn.sendOk(slow.id, Value.str("late"));
  conn.sendOk(next.id, Value.str("fresh"));

  // The pending entry was removed (CLT-020); the late response falls
  // under the unknown-id drop (CLT-013) and the connection lives on.
  expect(Value.asStr(await freshCall)).toBe("fresh");
  expect(client.unknownResponseDrops).toBe(1);
});

test("an AbortSignal cancels the call and removes the pending entry (CLT-021)", async () => {
  const server = await startServer();
  const client = await connect(server, plainConfig());
  const conn = await server.nextConn();

  const controller = new AbortController();
  const call = client.call("SLOW", [], { signal: controller.signal });
  void call.catch(() => undefined);
  const slow = await conn.nextRequest();

  controller.abort();
  let caught: unknown;
  try {
    await call;
  } catch (e) {
    caught = e;
  }
  // The rejection carries the signal's reason (AbortError by default).
  expect((caught as Error).name).toBe("AbortError");

  // The pending entry is gone: a late reply is an unknown-id drop and the
  // connection stays healthy.
  const freshCall = client.call("NEXT");
  const next = await conn.nextRequest();
  conn.sendOk(slow.id, Value.str("late"));
  conn.sendOk(next.id, Value.str("fresh"));
  expect(Value.asStr(await freshCall)).toBe("fresh");
  expect(client.unknownResponseDrops).toBe(1);

  // An already-aborted signal rejects before anything is sent.
  const preAborted = new AbortController();
  preAborted.abort();
  await expect(client.call("NOPE", [], { signal: preAborted.signal })).rejects.toMatchObject({
    name: "AbortError",
  });
});

// ── Reconnection (CLT-030/031) ──────────────────────────────────────────────

test("reconnect after a server drop succeeds", async () => {
  const server = await startServer();
  const client = await connect(server, plainConfig());
  const conn1 = await server.nextConn();

  const first = client.call("A");
  const requestA = await conn1.nextRequest();
  conn1.sendOk(requestA.id, Value.str("first"));
  expect(Value.asStr(await first)).toBe("first");

  conn1.destroy();
  // Let the reader observe the EOF and mark the connection dead.
  await sleep(200);

  // CLT-030: the call finds the connection dead and lazily re-dials.
  const second = client.call("B");
  const conn2 = await server.nextConn();
  const requestB = await conn2.nextRequest();
  conn2.sendOk(requestB.id, Value.str("second"));
  expect(Value.asStr(await second)).toBe("second");
});

test("a successful reconnect replays the handshake before pending traffic", async () => {
  const server = await startServer();
  const helloReply = Value.map([[Value.str("authenticated"), Value.bool(true)]]);
  // Not awaited yet: connect only resolves once the handshake below is
  // answered, so the server side has to be served first.
  const clientPromise = Client.connect(server.addr, helloMandatoryConfig(), {
    credentials: { type: "apiKey", apiKey: "k" },
  });
  void clientPromise.catch(() => undefined);

  const conn1 = await server.nextConn();
  const hello1 = await conn1.nextRequest();
  conn1.sendOk(hello1.id, helloReply);
  const client = await clientPromise;
  cleanups.push(() => client.close());

  const first = client.call("A");
  const requestA = await conn1.nextRequest();
  conn1.sendOk(requestA.id, Value.str("first"));
  expect(Value.asStr(await first)).toBe("first");

  conn1.destroy();
  // Let the reader observe the EOF and mark the connection dead.
  await sleep(200);

  const second = client.call("B");
  const conn2 = await server.nextConn();
  // What the re-dialed connection sees, in order: a client that skipped
  // the handshake would send only the call.
  const seen: string[] = [];
  const replayed = await conn2.nextRequest();
  seen.push(replayed.command);
  conn2.sendOk(replayed.id, helloReply);
  const requestB = await conn2.nextRequest();
  seen.push(requestB.command);
  conn2.sendOk(requestB.id, Value.str("second"));

  expect(Value.asStr(await second)).toBe("second");
  // CLT-030: the profile handshake is replayed before the pending call.
  expect(seen).toEqual(["HELLO", "B"]);
});

test("reconnect gives up after two attempts with a typed connection error", async () => {
  const server = await startServer();
  const clientPromise = Client.connect(server.addr, helloMandatoryConfig(), {
    credentials: { type: "apiKey", apiKey: "k" },
  });
  void clientPromise.catch(() => undefined);

  // Connection 1: serve the handshake and one call, then drop.
  const conn1 = await server.nextConn();
  const hello = await conn1.nextRequest();
  conn1.sendOk(hello.id, Value.map([[Value.str("authenticated"), Value.bool(true)]]));
  const client = await clientPromise;
  cleanups.push(() => client.close());

  const ping = client.call("PING");
  const request = await conn1.nextRequest();
  conn1.sendOk(request.id, Value.str("ok"));
  await ping;

  conn1.destroy();
  await sleep(200);

  // Re-dial attempts: accept and slam shut before the HelloMandatory
  // handshake can complete.
  const failing = client.call("PING");
  void failing.catch(() => undefined);
  const conn2 = await server.nextConn();
  conn2.destroy();
  const conn3 = await server.nextConn();
  conn3.destroy();

  let caught: unknown;
  try {
    await failing;
  } catch (e) {
    caught = e;
  }
  expect(
    caught,
    "expected the connection class after exhausted re-dials",
  ).toBeInstanceOf(ConnectionError);
  expect(server.accepts, "initial connect + exactly 2 re-dial attempts (CLT-030)").toBe(3);
});

// ── Error mapping (CLT-050..052) ────────────────────────────────────────────

test("RESP3 error mapping over the wire", async () => {
  const server = await startServer();
  const client = await connect(server, arglessHelloConfig());
  const conn = await server.nextConn();

  const get = client.call("GET");
  void get.catch(() => undefined);
  const getRequest = await conn.nextRequest();
  conn.sendErr(getRequest.id, "NOAUTH Authentication required.");
  let caught: unknown;
  try {
    await get;
  } catch (e) {
    caught = e;
  }
  expect(caught).toBeInstanceOf(AuthError);
  expect((caught as AuthError).message).toBe("NOAUTH Authentication required.");

  const foo = client.call("FOO");
  void foo.catch(() => undefined);
  const fooRequest = await conn.nextRequest();
  conn.sendErr(fooRequest.id, "ERR unknown command 'FOO'");
  try {
    await foo;
  } catch (e) {
    caught = e;
  }
  expect(caught).toBeInstanceOf(ServerError);
  expect((caught as ServerError).message).toBe("ERR unknown command 'FOO'");
  expect((caught as ServerError).code).toBeUndefined();

  // CLT-051: the *other* auth prefix is the auth class too — the unit table
  // pins the parser, this pins it end-to-end over a socket.
  const auth = client.call("AUTH");
  void auth.catch(() => undefined);
  const authRequest = await conn.nextRequest();
  conn.sendErr(authRequest.id, "WRONGPASS invalid username-password pair");
  try {
    await auth;
  } catch (e) {
    caught = e;
  }
  expect(caught).toBeInstanceOf(AuthError);
  expect((caught as AuthError).message).toBe("WRONGPASS invalid username-password pair");
});

test("bracket error mapping over the wire", async () => {
  const server = await startServer();
  const clientPromise = Client.connect(server.addr, helloMandatoryConfig(), {
    credentials: { type: "apiKey", apiKey: "k" },
  });
  void clientPromise.catch(() => undefined);

  const conn = await server.nextConn();
  const hello = await conn.nextRequest();
  conn.sendOk(hello.id, Value.map([[Value.str("authenticated"), Value.bool(true)]]));
  const client = await clientPromise;
  cleanups.push(() => client.close());

  const search = client.call("SEARCH");
  void search.catch(() => undefined);
  const request = await conn.nextRequest();
  conn.sendErr(request.id, "[collection_not_found] no such collection: docs");

  let caught: unknown;
  try {
    await search;
  } catch (e) {
    caught = e;
  }
  expect(caught).toBeInstanceOf(ServerError);
  expect((caught as ServerError).message).toBe(
    "[collection_not_found] no such collection: docs",
  );
  expect((caught as ServerError).code).toBe("collection_not_found");

  // CLT-051 says "regardless of convention": this config parses bracket
  // codes, not RESP3 prefixes, and the auth prefix must STILL win over the
  // wire rather than falling through to the server class.
  const auth = client.call("AUTH");
  void auth.catch(() => undefined);
  const authRequest = await conn.nextRequest();
  conn.sendErr(authRequest.id, "WRONGPASS invalid username-password pair");
  try {
    await auth;
  } catch (e) {
    caught = e;
  }
  expect(caught).toBeInstanceOf(AuthError);
  expect((caught as AuthError).message).toBe("WRONGPASS invalid username-password pair");
});

// ── Push frames (CLT-060) ───────────────────────────────────────────────────

test("push frames route to the handler under enabled", async () => {
  const server = await startServer();
  const client = await connect(server, plainConfig({ push: "enabled" }));
  const conn = await server.nextConn();

  const pushed: Value[] = [];
  client.onPush((value) => pushed.push(value));

  const call = client.call("SUBSCRIBE");
  const request = await conn.nextRequest();
  // A push frame in front of the response: it must reach the handler and
  // never be matched against the pending call.
  conn.sendOk(PUSH_ID, Value.str("evt"));
  conn.sendOk(request.id, Value.str("PONG"));

  expect(Value.asStr(await call)).toBe("PONG");
  expect(pushed.map((v) => Value.asStr(v))).toEqual(["evt"]);
  expect(client.unknownResponseDrops).toBe(0);
});

test("a push frame under a reserved config poisons the connection", async () => {
  const server = await startServer();
  const client = await connect(server, plainConfig());
  const conn1 = await server.nextConn();

  const call = client.call("GET");
  void call.catch(() => undefined);
  await conn1.nextRequest();
  conn1.sendOk(PUSH_ID, Value.null());

  let caught: unknown;
  try {
    await call;
  } catch (e) {
    caught = e;
  }
  expect(caught, "push under reserved is a protocol error (CLT-060)").toBeInstanceOf(DecodeError);

  // Poisoned connection, lazy reconnect on the next call (CLT-014/030).
  const recovered = client.call("GET");
  const conn2 = await server.nextConn();
  const request = await conn2.nextRequest();
  conn2.sendOk(request.id, Value.str("recovered"));
  expect(Value.asStr(await recovered)).toBe("recovered");
});

// ── Poisoning (CLT-014) ─────────────────────────────────────────────────────

test("an oversized inbound frame fails typed and poisons", async () => {
  const server = await startServer();
  const client = await connect(server, plainConfig({ maxFrameBytes: 64 }));
  const conn1 = await server.nextConn();

  const call = client.call("GET");
  void call.catch(() => undefined);
  await conn1.nextRequest();
  // A length prefix past the config cap — the client must refuse on the
  // prefix alone, before any body exists.
  const prefix = new Uint8Array(4);
  new DataView(prefix.buffer).setUint32(0, 1_000, true);
  conn1.sendRaw(prefix);

  let caught: unknown;
  try {
    await call;
  } catch (e) {
    caught = e;
  }
  expect(caught).toBeInstanceOf(FrameTooLargeError);

  const recovered = client.call("GET");
  const conn2 = await server.nextConn();
  const request = await conn2.nextRequest();
  conn2.sendOk(request.id, Value.str("recovered"));
  expect(Value.asStr(await recovered)).toBe("recovered");
});

test("a malformed frame poisons with a decode error", async () => {
  const server = await startServer();
  const client = await connect(server, plainConfig());
  const conn = await server.nextConn();

  const call = client.call("GET");
  void call.catch(() => undefined);
  await conn.nextRequest();
  // Valid length prefix, garbage body (0xc1 is never valid MessagePack).
  conn.sendRaw(Uint8Array.of(4, 0, 0, 0, 0xc1, 0xc1, 0xc1, 0xc1));

  let caught: unknown;
  try {
    await call;
  } catch (e) {
    caught = e;
  }
  expect(caught).toBeInstanceOf(DecodeError);
  expect(caught).toBeInstanceOf(ThunderError);
});

// ── Lifecycle (CLT-004) ─────────────────────────────────────────────────────

test("close is idempotent and fails in-flight calls", async () => {
  const server = await startServer();
  const client = await connect(server, plainConfig());
  const conn = await server.nextConn();

  const pending = client.call("HANG");
  void pending.catch(() => undefined);
  await conn.nextRequest(); // swallow the request, never answer

  await client.close();
  await client.close(); // idempotent (CLT-004)

  let caught: unknown;
  try {
    await pending;
  } catch (e) {
    caught = e;
  }
  expect(caught, "in-flight calls fail with the typed connection-closed error").toBeInstanceOf(
    ConnectionError,
  );
  expect((caught as ConnectionError).message).toBe("client is closed");

  await expect(client.call("AFTER")).rejects.toBeInstanceOf(ConnectionError);
});

// ── Endpoints (CLT-070) ─────────────────────────────────────────────────────

test("an HTTP URL is rejected at connect", async () => {
  let caught: unknown;
  try {
    await Client.connect("http://localhost:8080", plainConfig());
  } catch (e) {
    caught = e;
  }
  expect(caught).toBeInstanceOf(ConnectionError);
  const message = (caught as ConnectionError).message;
  expect(message).toContain("RPC-only");
  expect(message).toContain("HTTP client");
});
