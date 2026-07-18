/**
 * In-process loopback mock server for the client behavior suites
 * (SPEC-003). Built on the wire codec over real `node:net` sockets, it
 * stands in for `thunder-server` — the client contract is exercised
 * end-to-end. Serves plaintext by default; wraps the listener in a Node
 * `tls` server when {@link MockServerTls} is supplied, so the same request
 * queue drives both the plaintext and the encrypted transports.
 */

import * as net from "node:net";
import * as tls from "node:tls";

import {
  FrameReader,
  Response,
  Value,
  decodeRequestBody,
  encodeResponse,
} from "../src/index";
import type { Request } from "../src/index";

/** One accepted server-side connection with a request queue. */
export class ServerConn {
  readonly socket: net.Socket;
  readonly #reader = new FrameReader();
  readonly #requests: Request[] = [];
  readonly #waiters: ((request: Request) => void)[] = [];

  constructor(socket: net.Socket) {
    this.socket = socket;
    socket.on("data", (chunk) => {
      this.#reader.push(chunk);
      for (;;) {
        const body = this.#reader.nextBody();
        if (body === null) break;
        const request = decodeRequestBody(body);
        const waiter = this.#waiters.shift();
        if (waiter) waiter(request);
        else this.#requests.push(request);
      }
    });
    socket.on("error", () => {
      // Client teardown surfaces as ECONNRESET here; irrelevant.
    });
  }

  nextRequest(): Promise<Request> {
    const queued = this.#requests.shift();
    if (queued) return Promise.resolve(queued);
    return new Promise((resolve) => this.#waiters.push(resolve));
  }

  /** Requests received but not yet pulled — the backpressure probe. */
  get queuedRequests(): number {
    return this.#requests.length;
  }

  sendOk(id: number, value: Value): void {
    this.socket.write(encodeResponse(Response.ok(id, value)));
  }

  sendErr(id: number, message: string): void {
    this.socket.write(encodeResponse(Response.err(id, message)));
  }

  sendRaw(bytes: Uint8Array): void {
    this.socket.write(bytes);
  }

  destroy(): void {
    this.socket.destroy();
  }
}

/** Server-side TLS material for {@link MockServer.listen}. */
export interface MockServerTls {
  /** PEM certificate chain. */
  cert: string | Buffer;
  /** PEM private key. */
  key: string | Buffer;
}

/** In-process loopback mock server. */
export class MockServer {
  accepts = 0;
  readonly #server: net.Server;
  readonly #port: number;
  readonly #conns: ServerConn[] = [];
  readonly #queue: ServerConn[] = [];
  readonly #waiters: ((conn: ServerConn) => void)[] = [];

  private constructor(server: net.Server, port: number) {
    this.#server = server;
    this.#port = port;
  }

  static listen(options: { tls?: MockServerTls } = {}): Promise<MockServer> {
    return new Promise((resolve, reject) => {
      const server = options.tls
        ? tls.createServer({ cert: options.tls.cert, key: options.tls.key })
        : net.createServer();
      server.once("error", reject);
      if (options.tls) {
        // An untrusted-cert client (the mismatch test) fails the handshake
        // here; swallow it so it is not an unhandled event.
        server.on("tlsClientError", () => undefined);
      }
      server.listen(0, "127.0.0.1", () => {
        const address = server.address();
        if (address === null || typeof address === "string") {
          reject(new Error("no bound address"));
          return;
        }
        const instance = new MockServer(server, address.port);
        // Plaintext accepts fire on "connection"; a TLS listener yields the
        // already-handshaked `TLSSocket` on "secureConnection".
        server.on(options.tls ? "secureConnection" : "connection", (socket: net.Socket) => {
          instance.#accept(socket);
        });
        resolve(instance);
      });
    });
  }

  #accept(socket: net.Socket): void {
    this.accepts += 1;
    const conn = new ServerConn(socket);
    this.#conns.push(conn);
    const waiter = this.#waiters.shift();
    if (waiter) waiter(conn);
    else this.#queue.push(conn);
  }

  get addr(): string {
    return `127.0.0.1:${this.#port}`;
  }

  nextConn(): Promise<ServerConn> {
    const queued = this.#queue.shift();
    if (queued) return Promise.resolve(queued);
    return new Promise((resolve) => this.#waiters.push(resolve));
  }

  close(): Promise<void> {
    for (const conn of this.#conns) conn.destroy();
    return new Promise((resolve) => {
      this.#server.close(() => resolve());
    });
  }
}
