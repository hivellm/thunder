/**
 * Endpoint parsing (CLT-070/071) — mirrors the Rust
 * `thunder/src/client/endpoint.rs` test suite exactly.
 */

import { expect, test } from "vitest";

import { Config, ConnectionError, parseEndpoint } from "../src/index";

/**
 * An application's config — Thunder ships no schemes of its own, so the
 * tests bring their own, exactly as an application does.
 */
function app(): Config {
  return Config.standard().withScheme("myapp").withPort(9000);
}

function messageOf(fn: () => unknown): string {
  try {
    fn();
  } catch (e) {
    expect(e).toBeInstanceOf(ConnectionError);
    return (e as ConnectionError).message;
  }
  throw new Error("expected the parse to fail");
}

test("the configured scheme resolves the configured default port (CLT-071)", () => {
  // CLT-071: scheme → default port comes from the application's own config,
  // not from any registry Thunder carries.
  const endpoint = parseEndpoint("myapp://db.example.com", app());
  expect(endpoint.host).toBe("db.example.com");
  expect(endpoint.port).toBe(9000);
});

test("any application can pick any scheme without a Thunder release", () => {
  // The whole point of dropping the registry: a scheme Thunder has never
  // heard of works because the application configured it.
  const future = Config.standard().withScheme("something-new-in-2030").withPort(4242);
  expect(parseEndpoint("something-new-in-2030://host", future).port).toBe(4242);
});

test("an explicit port wins over the default", () => {
  expect(parseEndpoint("myapp://10.0.0.7:9999", app())).toEqual({
    host: "10.0.0.7",
    port: 9999,
  });
});

test("bare host:port is accepted, RPC implied", () => {
  expect(parseEndpoint("localhost:15501", app())).toEqual({
    host: "localhost",
    port: 15501,
  });
});

test("bare host:port works even with no scheme configured", () => {
  // Config.standard() has no identity until an application gives it one; an
  // explicit host:port needs none.
  expect(parseEndpoint("localhost:15501", Config.standard()).port).toBe(15501);
});

test("bare host without a port is rejected", () => {
  expect(() => parseEndpoint("localhost", app())).toThrow(ConnectionError);
});

test("http and https are rejected with a pointer to the HTTP client (CLT-070)", () => {
  for (const url of ["http://db.example.com:8080", "https://db.example.com"]) {
    const message = messageOf(() => parseEndpoint(url, app()));
    expect(message, url).toContain("RPC-only");
    expect(message, url).toContain("HTTP client");
  }
});

test("a scheme other than the configured one is rejected", () => {
  const message = messageOf(() => parseEndpoint("redis://h:1", app()));
  // The mismatch must name both the given and the configured scheme.
  expect(message).toContain("redis");
  expect(message).toContain("myapp");
});

test("IPv6 literals parse with and without brackets", () => {
  expect(parseEndpoint("[::1]:8080", app())).toEqual({ host: "::1", port: 8080 });
  const endpoint = parseEndpoint("myapp://[fe80::1]", app());
  expect(endpoint.host).toBe("fe80::1");
  expect(endpoint.port).toBe(9000);
});

test("a trailing slash is tolerated but paths are not", () => {
  expect(parseEndpoint("myapp://h/", app()).port).toBe(9000);
  expect(() => parseEndpoint("myapp://h/db", app())).toThrow(ConnectionError);
});

test("invalid ports are rejected", () => {
  expect(() => parseEndpoint("host:99999", app())).toThrow(ConnectionError);
  expect(() => parseEndpoint("myapp://host:abc", app())).toThrow(ConnectionError);
});

test("an empty host is rejected", () => {
  expect(() => parseEndpoint("myapp://:1234", app())).toThrow(ConnectionError);
  expect(() => parseEndpoint(":1234", app())).toThrow(ConnectionError);
});
