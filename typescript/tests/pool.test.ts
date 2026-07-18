/**
 * Connection-pool behavior (CLT-080), mirroring the Rust suite
 * (`rust/thunder/tests/pool.rs`). The pool is a layer above the
 * single-connection client; these tests exercise it end to end over the
 * loopback mock server — checkout/return, the capacity bound, the release on
 * the error path, the poison drop, and the property the whole layer exists
 * for: `N` operations pay **one** connection and **one** handshake, not `N`
 * (counted server-side via distinct accepted connections).
 */

import { afterEach, expect, test } from "vitest";

import { Config, Pool, ServerError, Value } from "../src/index";

import { MockServer, ServerConn } from "./mock-server";

/** The standard config (mandatory HELLO map + capabilities reply), so a real
 * handshake happens on every new connection. */
function profile(): Config {
  return Config.standard().withScheme("test").withPort(0);
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Auto-reply to one connection's frames: HELLO → authenticated, PING → PONG,
 * anything else → a server ERR. Lets `Client.connect`'s handshake complete
 * without the test hand-servicing each frame. */
function serve(conn: ServerConn): void {
  void (async () => {
    for (;;) {
      const req = await conn.nextRequest();
      if (req.command === "HELLO") {
        conn.sendOk(
          req.id,
          Value.map([
            [Value.str("authenticated"), Value.bool(true)],
            [Value.str("capabilities"), Value.array([])],
          ]),
        );
      } else if (req.command === "PING") {
        conn.sendOk(req.id, Value.str("PONG"));
      } else {
        conn.sendErr(req.id, `ERR unknown command '${req.command}'`);
      }
    }
  })();
}

/** Accept and auto-serve every connection the pool dials. */
function autoServe(server: MockServer): void {
  void (async () => {
    for (;;) {
      serve(await server.nextConn());
    }
  })();
}

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  while (cleanups.length > 0) {
    const cleanup = cleanups.pop();
    if (cleanup) await cleanup();
  }
});

async function serving(): Promise<MockServer> {
  const server = await MockServer.listen();
  cleanups.push(() => server.close());
  autoServe(server);
  return server;
}

test("checkout returns the connection for reuse", async () => {
  const server = await serving();
  const pool = new Pool(server.addr, profile(), { clientName: "pool-test" }, 4);

  expect(pool.idleCount, "construction dials nothing").toBe(0);

  const conn = await pool.acquire();
  expect(Value.asStr(await conn.client.call("PING"))).toBe("PONG");
  expect(pool.idleCount, "checked out, so not idle").toBe(0);

  conn.release();
  expect(pool.idleCount, "returned on release").toBe(1);
});

test("N operations use one connection and one handshake", async () => {
  const server = await serving();
  const pool = new Pool(server.addr, profile(), { clientName: "pool-test" }, 4);

  for (let i = 0; i < 10; i += 1) {
    const conn = await pool.acquire();
    expect(Value.asStr(await conn.client.call("PING"))).toBe("PONG");
    conn.release();
  }

  // The whole point of the layer: ten sequential operations reused one
  // connection, so the server saw one handshake, not ten.
  expect(server.accepts, "ten operations must ride one connection (one handshake)").toBe(1);
});

test("the pool never exceeds maxConnections under concurrent checkout", async () => {
  const server = await serving();
  const pool = new Pool(server.addr, profile(), { clientName: "pool-test" }, 2);

  const a = await pool.acquire();
  const b = await pool.acquire();

  // With both permits held, a third checkout must wait, not open a third
  // connection (CLT-080 fixed N).
  let thirdResolved = false;
  const third = pool.acquire().then((conn) => {
    thirdResolved = true;
    return conn;
  });
  await sleep(150);
  expect(thirdResolved, "third checkout must block while max are held").toBe(false);
  expect(server.accepts, "no third connection while two are held").toBe(2);

  // Release one; the waiter now completes, reusing the freed connection.
  a.release();
  const c = await third;
  expect(thirdResolved).toBe(true);
  expect(Value.asStr(await c.client.call("PING"))).toBe("PONG");
  expect(server.accepts, "at most two connections ever existed").toBe(2);

  b.release();
  c.release();
});

test("release on the error path still returns the connection", async () => {
  const server = await serving();
  const pool = new Pool(server.addr, profile(), { clientName: "pool-test" }, 4);

  const conn = await pool.acquire();
  let caught: unknown;
  try {
    await conn.client.call("BOOM"); // server replies ERR → ServerError
  } catch (e) {
    caught = e;
  } finally {
    conn.release();
  }
  // A server-level error does not poison the transport: the connection is
  // still live and returns for reuse.
  expect(caught).toBeInstanceOf(ServerError);
  expect(pool.idleCount, "a live connection returns even on the error path").toBe(1);
});

test("a poisoned connection is not handed to the next caller", async () => {
  const server = await serving();
  const pool = new Pool(server.addr, profile(), { clientName: "pool-test" }, 4);

  const conn = await pool.acquire();
  expect(Value.asStr(await conn.client.call("PING"))).toBe("PONG");
  // Kill this connection, then release the guard.
  await conn.client.close();
  expect(conn.client.isAlive).toBe(false);
  conn.release();

  // CLT-014: the dead connection was dropped, not parked for reuse.
  expect(pool.idleCount, "a poisoned connection must not return to the pool").toBe(0);

  // The next checkout dials a fresh, working connection.
  const fresh = await pool.acquire();
  expect(Value.asStr(await fresh.client.call("PING"))).toBe("PONG");
  fresh.release();
  expect(server.accepts, "the fresh checkout dialed a new connection").toBe(2);
});
