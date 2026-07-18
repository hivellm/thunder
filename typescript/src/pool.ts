/**
 * Optional connection pool (CLT-080) — a layer **above** the
 * single-connection {@link Client} (CLT-001: "pooling is a layer above").
 *
 * Under a mandatory-`HELLO` config with `auth_required`, a fresh connection
 * costs a handshake round trip before the first request. A caller that opens a
 * connection per operation therefore pays that round trip every time. The pool
 * amortizes it: `N` operations over a checked-out connection pay **one**
 * connect and **one** handshake, not `N`.
 *
 * The shape is deliberately minimal — a fixed number of connections bounded by
 * a semaphore, an idle list, lazy connect on first checkout, and a guard that
 * returns the connection on {@link PooledConn.release}. It is **not** an
 * external pool library: health checks, background reaping and min-idle warmup
 * are out of scope; a poisoned connection (CLT-014) is dropped on release and
 * the next checkout connects fresh, leaving reconnect to CLT-030 rather than
 * the pool.
 *
 * The pool adds **no wire behavior**: it builds the same {@link Client} as
 * {@link Client.connect} from a {@link Config} and {@link ClientOptions}, and
 * the single-connection client's API is unchanged (CLT-001). `maxInFlight`
 * (CLT-012) stays a per-connection bound; the pool bounds connections, not
 * in-flight calls.
 *
 * TypeScript has no RAII drop, so the guard is explicit: call
 * {@link PooledConn.release} in a `finally`.
 *
 * ```ts
 * import { Config, Pool } from "@hivehub/thunder";
 *
 * const app = Config.standard().withScheme("myapp").withPort(9000);
 * const pool = new Pool("myapp://localhost", app, {}, 8);
 * const conn = await pool.acquire(); // reuses an idle connection, or dials one
 * try {
 *   const pong = await conn.client.call("PING");
 * } finally {
 *   conn.release(); // returns the connection to the pool for reuse
 * }
 * ```
 */

import { Client } from "./client";
import type { ClientOptions } from "./client";
import type { Config } from "./config";

/**
 * Bounds live + checked-out connections (CLT-080 "fixed N"). A checkout beyond
 * the limit awaits a release rather than opening another connection. FIFO, so
 * waiters proceed in order.
 */
class PoolSemaphore {
  #available: number;
  readonly #waiters: (() => void)[] = [];

  constructor(permits: number) {
    this.#available = permits;
  }

  acquire(): Promise<void> {
    if (this.#available > 0) {
      this.#available -= 1;
      return Promise.resolve();
    }
    return new Promise((resolve) => this.#waiters.push(resolve));
  }

  release(): void {
    const next = this.#waiters.shift();
    if (next) next();
    else this.#available += 1;
  }
}

/**
 * A bounded pool of {@link Client}s over one endpoint (CLT-080).
 *
 * At most `maxConnections` connections are live at once; a checkout beyond that
 * awaits a return. Connections are dialed lazily — construction opens none —
 * and reused across checkouts so the handshake is paid once per connection, not
 * once per operation.
 */
export class Pool {
  readonly #endpoint: string;
  readonly #config: Config;
  readonly #options: ClientOptions;
  readonly #permits: PoolSemaphore;
  /** Idle connections available for reuse. */
  readonly #idle: Client[] = [];

  /**
   * Build a pool for `endpoint`. Opens no connections — the first
   * {@link acquire} dials the first one. `maxConnections` is clamped to at
   * least 1.
   */
  constructor(
    endpoint: string,
    config: Config,
    options: ClientOptions = {},
    maxConnections = 1,
  ) {
    this.#endpoint = endpoint;
    this.#config = config;
    this.#options = options;
    this.#permits = new PoolSemaphore(Math.max(1, Math.floor(maxConnections)));
  }

  /**
   * Check out a connection. Reuses an idle, **live** connection when one is
   * available; otherwise dials and handshakes a fresh one (CLT-002). Awaits a
   * return when `maxConnections` are already checked out. The returned
   * {@link PooledConn} returns the connection to the pool on
   * {@link PooledConn.release}.
   */
  async acquire(): Promise<PooledConn> {
    await this.#permits.acquire();
    // Reuse the newest idle connection that is still live; discard any that
    // were poisoned (CLT-014) while sitting idle.
    let client: Client | null = null;
    for (;;) {
      const candidate = this.#idle.pop();
      if (candidate === undefined) break;
      if (candidate.isAlive) {
        client = candidate;
        break;
      }
      void candidate.close();
    }
    if (client === null) {
      try {
        client = await Client.connect(this.#endpoint, this.#config, this.#options);
      } catch (error) {
        // The dial failed: give the permit back so it is not leaked.
        this.#permits.release();
        throw error;
      }
    }
    return new PooledConn(client, this.#idle, this.#permits);
  }

  /**
   * Idle connections currently parked in the pool. For diagnostics and tests —
   * production code should not branch on it.
   */
  get idleCount(): number {
    return this.#idle.length;
  }
}

/**
 * Guard from {@link Pool.acquire}. Exposes the checked-out {@link Client} via
 * {@link client} and returns the connection to the pool on {@link release} so
 * the next checkout reuses it — unless the connection was poisoned, in which
 * case it is dropped and the next checkout connects fresh (CLT-014/030).
 *
 * TypeScript has no RAII drop, so {@link release} is explicit: call it in a
 * `finally`. It is idempotent — a second call is a no-op.
 */
export class PooledConn {
  #client: Client | null;
  readonly #idle: Client[];
  readonly #permits: PoolSemaphore;

  /** @internal — constructed only by {@link Pool.acquire}. */
  constructor(client: Client, idle: Client[], permits: PoolSemaphore) {
    this.#client = client;
    this.#idle = idle;
    this.#permits = permits;
  }

  /** The checked-out client. Throws after {@link release}. */
  get client(): Client {
    if (this.#client === null) {
      throw new Error("PooledConn used after release()");
    }
    return this.#client;
  }

  /**
   * Return the connection to the pool (idempotent). CLT-014: only a live
   * connection is parked for reuse; a poisoned or closed one is dropped here
   * and the next checkout dials fresh, leaving reconnect to CLT-030 rather
   * than the pool. Releasing the permit lets a waiting checkout proceed.
   */
  release(): void {
    const client = this.#client;
    if (client === null) return;
    this.#client = null;
    if (client.isAlive) {
      this.#idle.push(client);
    } else {
      void client.close();
    }
    this.#permits.release();
  }
}
