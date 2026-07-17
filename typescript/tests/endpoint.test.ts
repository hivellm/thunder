/**
 * Endpoint parsing (CLT-070/071) — mirrors the Rust
 * `thunder-client/src/endpoint.rs` test suite exactly.
 */

import { expect, test } from "vitest";

import { ConnectionError, Profiles, parseEndpoint } from "../src/index";

function messageOf(fn: () => unknown): string {
  try {
    fn();
  } catch (e) {
    expect(e).toBeInstanceOf(ConnectionError);
    return (e as ConnectionError).message;
  }
  throw new Error("expected the parse to fail");
}

test("every registered scheme resolves its default port (CLT-071)", () => {
  for (const profile of Profiles.registry()) {
    const endpoint = parseEndpoint(`${profile.scheme}://db.example.com`);
    expect(endpoint.host, profile.scheme).toBe("db.example.com");
    expect(endpoint.port, profile.scheme).toBe(profile.defaultPort);
  }
});

test("an explicit port wins over the default", () => {
  expect(parseEndpoint("nexus://10.0.0.7:9999")).toEqual({ host: "10.0.0.7", port: 9999 });
});

test("bare host:port is accepted, RPC implied", () => {
  expect(parseEndpoint("localhost:15501")).toEqual({ host: "localhost", port: 15501 });
});

test("bare host without a port is rejected", () => {
  expect(() => parseEndpoint("localhost")).toThrow(ConnectionError);
});

test("http and https are rejected with a pointer to the HTTP client (CLT-070)", () => {
  for (const url of ["http://vec.example.com:8080", "https://vec.example.com"]) {
    const message = messageOf(() => parseEndpoint(url));
    expect(message, url).toContain("RPC-only");
    expect(message, url).toContain("HTTP client");
  }
});

test("an unknown scheme is rejected listing the registry", () => {
  const message = messageOf(() => parseEndpoint("redis://h:1"));
  for (const scheme of ["synap", "nexus", "vectorizer", "lexum"]) {
    expect(message).toContain(scheme);
  }
});

test("IPv6 literals parse with and without brackets", () => {
  expect(parseEndpoint("[::1]:8080")).toEqual({ host: "::1", port: 8080 });
  const endpoint = parseEndpoint("synap://[fe80::1]");
  expect(endpoint.host).toBe("fe80::1");
  expect(endpoint.port).toBe(Profiles.synap.defaultPort);
});

test("a trailing slash is tolerated but paths are not", () => {
  expect(parseEndpoint("lexum://h/").port).toBe(Profiles.lexum.defaultPort);
  expect(() => parseEndpoint("lexum://h/db")).toThrow(ConnectionError);
});

test("invalid ports are rejected", () => {
  expect(() => parseEndpoint("host:99999")).toThrow(ConnectionError);
  expect(() => parseEndpoint("synap://host:abc")).toThrow(ConnectionError);
  expect(() => parseEndpoint(":1234")).toThrow(ConnectionError);
});
