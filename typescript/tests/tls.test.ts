/**
 * Optional-TLS transport tests (SPEC-008 CAN-020, FR-29), mirroring the Rust
 * suite (`rust/thunder/tests/tls.rs`). Three properties: an encrypted round
 * trip actually works end to end; the plaintext path is unchanged when TLS is
 * unused; and a cert the client does not trust fails as a
 * {@link ConnectionError}, not a hang or a throw of another class.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { afterEach, expect, test } from "vitest";

import { Client, ConnectionError, Value } from "../src/index";
import type { Config } from "../src/index";

import { MockServer } from "./mock-server";
import { OTHER_CERT, SERVER_CERT, SERVER_KEY } from "./tls-certs";

/**
 * A no-handshake config — the tests exercise the transport, not auth. `tls`
 * here is only the wire-config *policy* signal (PRO-003: config is data); the
 * actual transport TLS is driven by the client options below.
 */
function profile(overrides: Partial<Config> = {}): Config {
  return {
    scheme: "test",
    defaultPort: 0,
    handshake: "none",
    helloStyle: "not_used",
    push: "reserved",
    maxFrameBytes: 1024 * 1024,
    maxInFlight: 64,
    errorCodes: "none",
    tls: "optional",
    ...overrides,
  };
}

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  while (cleanups.length > 0) {
    const cleanup = cleanups.pop();
    if (cleanup) await cleanup();
  }
});

let tmpCounter = 0;
/** Write PEM to a uniquely-named temp file (a `caPath` is a filesystem path). */
function writeTempPem(contents: string): string {
  const file = path.join(os.tmpdir(), `thunder-ts-tls-${process.pid}-${tmpCounter++}.pem`);
  fs.writeFileSync(file, contents);
  cleanups.push(() => {
    try {
      fs.rmSync(file);
    } catch {
      /* best-effort cleanup */
    }
  });
  return file;
}

test("a TLS round trip encrypts request and response", async () => {
  const server = await MockServer.listen({ tls: { cert: SERVER_CERT, key: SERVER_KEY } });
  cleanups.push(() => server.close());

  // The client trusts exactly this self-signed cert and verifies the SAN
  // `localhost`.
  const caPath = writeTempPem(SERVER_CERT);
  const client = await Client.connect(server.addr, profile(), {
    tls: { serverName: "localhost", caPath },
  });
  cleanups.push(() => client.close());
  const conn = await server.nextConn();

  const ping = client.call("PING");
  const pingReq = await conn.nextRequest();
  expect(pingReq.command).toBe("PING");
  conn.sendOk(pingReq.id, Value.str("PONG"));
  expect(Value.asStr(await ping)).toBe("PONG");

  const echo = client.call("ECHO", [Value.str("secret-over-tls")]);
  const echoReq = await conn.nextRequest();
  conn.sendOk(echoReq.id, echoReq.args[0] ?? Value.null());
  expect(Value.asStr(await echo)).toBe("secret-over-tls");
});

test("plaintext still works when TLS is unused", async () => {
  // Same client stack, no TLS on either end — the default path is unchanged.
  const server = await MockServer.listen();
  cleanups.push(() => server.close());
  const client = await Client.connect(server.addr, profile({ tls: "off" }));
  cleanups.push(() => client.close());
  const conn = await server.nextConn();

  const ping = client.call("PING");
  const req = await conn.nextRequest();
  conn.sendOk(req.id, Value.str("PONG"));
  expect(Value.asStr(await ping)).toBe("PONG");
});

test("a cert mismatch is a ConnectionError", async () => {
  const server = await MockServer.listen({ tls: { cert: SERVER_CERT, key: SERVER_KEY } });
  cleanups.push(() => server.close());

  // A DIFFERENT self-signed cert the client trusts instead of the server's —
  // verification must fail.
  const wrongCa = writeTempPem(OTHER_CERT);
  let caught: unknown;
  try {
    await Client.connect(server.addr, profile(), {
      tls: { serverName: "localhost", caPath: wrongCa },
    });
  } catch (e) {
    caught = e;
  }
  // FR-29: a TLS/verification failure is the Connection class.
  expect(caught, `expected a ConnectionError from an untrusted cert, got ${String(caught)}`)
    .toBeInstanceOf(ConnectionError);
});
