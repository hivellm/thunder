/**
 * Pins the `Profiles` registry constants to the language-neutral data
 * files in `conformance/profiles/` (PRO-010/013): a registry edit that is
 * not mirrored in the YAML — or vice versa — fails here.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { expect, test } from "vitest";
import { parse } from "yaml";

import { Profiles } from "../src/index";
import type {
  ErrorConvention,
  Handshake,
  HelloStyle,
  PushPolicy,
  TlsPolicy,
} from "../src/index";

const PROFILES_DIR = fileURLToPath(
  new URL("../../conformance/profiles/", import.meta.url),
);

interface RawProfile {
  name: string;
  scheme: string;
  default_port: number;
  handshake: string;
  hello_style: string | null;
  push: string;
  max_frame_bytes: number;
  max_in_flight: number;
  error_codes: string;
  tls: string;
}

function mapHandshake(raw: string): Handshake {
  switch (raw) {
    case "none":
    case "auth_command":
    case "hello_mandatory":
      return raw;
    default:
      throw new Error(`unknown handshake ${raw}`);
  }
}

function mapHelloStyle(raw: string | null): HelloStyle {
  switch (raw) {
    case null:
      return "not_used";
    case "positional_version":
    case "map_payload":
      return raw;
    default:
      throw new Error(`unknown hello_style ${raw}`);
  }
}

function mapPush(raw: string): PushPolicy {
  switch (raw) {
    case "reserved":
    case "enabled":
      return raw;
    default:
      throw new Error(`unknown push ${raw}`);
  }
}

function mapErrors(raw: string): ErrorConvention {
  switch (raw) {
    case "none":
    case "resp3_prefixes":
    case "bracket_code":
    case "both":
      return raw;
    default:
      throw new Error(`unknown error_codes ${raw}`);
  }
}

function mapTls(raw: string): TlsPolicy {
  switch (raw) {
    case "off":
      return "off";
    case "optional_rustls":
      return "optional";
    case "reserved_config":
      return "reserved";
    default:
      throw new Error(`unknown tls ${raw}`);
  }
}

test("registry constants match conformance/profiles (PRO-010/013)", () => {
  let matched = 0;
  for (const profile of Profiles.registry()) {
    const raw = readFileSync(join(PROFILES_DIR, `${profile.name}.yaml`), "utf8");
    const y = parse(raw) as RawProfile;
    expect(y.name, profile.name).toBe(profile.name);
    expect(y.scheme, profile.name).toBe(profile.scheme);
    expect(y.default_port, profile.name).toBe(profile.defaultPort);
    expect(y.max_frame_bytes, profile.name).toBe(profile.maxFrameBytes);
    expect(y.max_in_flight, profile.name).toBe(profile.maxInFlight);
    expect(mapHandshake(y.handshake), profile.name).toBe(profile.handshake);
    expect(mapHelloStyle(y.hello_style), profile.name).toBe(profile.helloStyle);
    expect(mapPush(y.push), profile.name).toBe(profile.push);
    expect(mapErrors(y.error_codes), profile.name).toBe(profile.errorCodes);
    expect(mapTls(y.tls), profile.name).toBe(profile.tls);
    matched += 1;
  }
  expect(matched, "all four family profiles pinned").toBe(4);
});

test("named constants and the registry agree", () => {
  expect(Profiles.registry()).toEqual([
    Profiles.synap,
    Profiles.nexus,
    Profiles.vectorizer,
    Profiles.lexum,
  ]);
});

test("custom profile construction stays open (PRO-020)", () => {
  const custom = { ...Profiles.vectorizer, name: "acme", scheme: "acme", defaultPort: 9000 };
  expect(custom.handshake).toBe("hello_mandatory");
  expect(custom.defaultPort).toBe(9000);
  // The registry constant itself is immutable.
  expect(Object.isFrozen(Profiles.vectorizer)).toBe(true);
});
