/**
 * Endpoint parsing (CLT-070/071).
 *
 * Accepts `scheme://host[:port]` for every scheme in the profile
 * registry — scheme → default-port resolution is data-driven (PRO-012),
 * products never fork the parser — plus bare `host:port` (RPC implied;
 * the caller supplies the profile). `http(s)://` URLs are rejected with a
 * pointer to the product's HTTP client: Thunder is RPC-only.
 *
 * Parse failures use the {@link ConnectionError} class — an endpoint that
 * cannot be parsed is an endpoint that cannot be dialed.
 */

import { ConnectionError } from "./errors";
import { Profiles } from "./profile";

/** A resolved RPC endpoint: host plus concrete port. */
export interface Endpoint {
  /** Host name or IP literal (IPv6 without brackets). */
  host: string;
  /** Concrete port — explicit, or the scheme's registry default. */
  port: number;
}

/**
 * Parse an endpoint string (CLT-070).
 *
 * Accepted forms:
 * - `scheme://host[:port]` for every registered profile scheme
 *   (`synap`, `nexus`, `vectorizer`, `lexum`); a missing port resolves to
 *   the scheme's registry default (CLT-071).
 * - bare `host:port` (RPC implied — the caller supplies the profile).
 * - `[v6::addr]:port` / `scheme://[v6::addr][:port]` for IPv6 literals.
 *
 * `http://` / `https://` are rejected: Thunder is RPC-only; REST
 * endpoints belong to the product's HTTP client.
 */
export function parseEndpoint(input: string): Endpoint {
  const trimmed = input.trim();
  const schemeSplit = trimmed.indexOf("://");
  if (schemeSplit >= 0) {
    const scheme = trimmed.slice(0, schemeSplit).toLowerCase();
    let rest = trimmed.slice(schemeSplit + 3);
    if (scheme === "http" || scheme === "https") {
      throw invalid(
        `'${trimmed}' is an HTTP URL and Thunder is RPC-only — use the product's HTTP ` +
          `client for REST endpoints, or pass an RPC endpoint such as ` +
          `'vectorizer://host:port' or bare 'host:port'`,
      );
    }
    const profile = Profiles.registry().find((p) => p.scheme === scheme);
    if (profile === undefined) {
      const known = Profiles.registry()
        .map((p) => p.scheme)
        .join(", ");
      throw invalid(
        `unknown endpoint scheme '${scheme}' — registered schemes: ${known}; ` +
          `or use bare 'host:port'`,
      );
    }
    if (rest.endsWith("/")) rest = rest.slice(0, -1);
    if (rest.includes("/")) {
      throw invalid(
        `endpoint '${trimmed}' must not carry a path — expected ${scheme}://host[:port]`,
      );
    }
    const { host, port } = splitHostPort(rest);
    return { host, port: port ?? profile.defaultPort };
  }
  const { host, port } = splitHostPort(trimmed);
  if (port === undefined) {
    throw invalid(
      `bare endpoint '${trimmed}' needs an explicit port ('host:port') — only ` +
        `scheme-prefixed endpoints resolve a registry default port`,
    );
  }
  return { host, port };
}

/** Split `host[:port]`, handling bracketed IPv6 literals. */
function splitHostPort(s: string): { host: string; port: number | undefined } {
  if (s === "") {
    throw invalid("endpoint host is empty");
  }
  if (s.startsWith("[")) {
    const end = s.indexOf("]");
    if (end < 0) {
      throw invalid(`unterminated '[' in endpoint host '${s}'`);
    }
    const host = s.slice(1, end);
    if (host === "") {
      throw invalid("endpoint host is empty");
    }
    const tail = s.slice(end + 1);
    if (tail === "") return { host, port: undefined };
    if (!tail.startsWith(":")) {
      throw invalid(`expected ':port' after ']' in endpoint '${s}'`);
    }
    return { host, port: parsePort(tail.slice(1), s) };
  }
  const colon = s.lastIndexOf(":");
  if (colon < 0) {
    return { host: s, port: undefined };
  }
  const head = s.slice(0, colon);
  if (head.includes(":")) {
    // More than one ':' without brackets: an IPv6 literal, no port.
    return { host: s, port: undefined };
  }
  if (head === "") {
    throw invalid("endpoint host is empty");
  }
  return { host: head, port: parsePort(s.slice(colon + 1), s) };
}

function parsePort(port: string, whole: string): number {
  if (!/^\d+$/.test(port)) {
    throw invalid(`invalid port '${port}' in endpoint '${whole}'`);
  }
  const value = Number(port);
  if (value > 0xffff) {
    throw invalid(`invalid port '${port}' in endpoint '${whole}'`);
  }
  return value;
}

function invalid(message: string): ConnectionError {
  return new ConnectionError(message);
}
