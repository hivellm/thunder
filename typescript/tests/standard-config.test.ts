/**
 * Pins `Config.standard()` to `conformance/standard.yaml` (PRO-013).
 *
 * Thunder ships **one** standard and no product knowledge, so this is the
 * whole registry check: a change to the standard that is not mirrored in
 * the language-neutral YAML — or vice versa — fails here, in all four
 * languages. That cross-language agreement was the only job the old
 * per-product registry legitimately did; it survives without any product
 * name.
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { expect, test } from "vitest";
import { parse } from "yaml";

import { Config, DEFAULT_MAX_FRAME_BYTES } from "../src/index";
import type {
  ErrorConvention,
  Handshake,
  HelloStyle,
  PushPolicy,
  TlsPolicy,
} from "../src/index";

const STANDARD_YAML = fileURLToPath(
  new URL("../../conformance/standard.yaml", import.meta.url),
);

interface RawStandard {
  handshake: string;
  hello_style: string;
  push: string;
  max_frame_bytes: number;
  max_in_flight: number;
  error_codes: string;
  tls: string;
}

function standardYaml(): RawStandard {
  return parse(readFileSync(STANDARD_YAML, "utf8")) as RawStandard;
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

function mapHelloStyle(raw: string): HelloStyle {
  switch (raw) {
    case "not_used":
    case "arg_less":
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

test("the standard matches the conformance data file (PRO-013)", () => {
  const y = standardYaml();
  const s = Config.standard();

  expect(mapHandshake(y.handshake)).toBe(s.handshake);
  expect(mapHelloStyle(y.hello_style)).toBe(s.helloStyle);
  expect(mapPush(y.push)).toBe(s.push);
  expect(y.max_frame_bytes).toBe(s.maxFrameBytes);
  expect(y.max_in_flight).toBe(s.maxInFlight);
  expect(mapErrors(y.error_codes)).toBe(s.errorCodes);
  expect(mapTls(y.tls)).toBe(s.tls);
});

test("the standard carries no identity", () => {
  // Identity is the application's: Thunder has no opinion about which
  // scheme or port an implementation answers on.
  const s = Config.standard();
  expect(s.scheme).toBe("");
  expect(s.defaultPort).toBe(0);
});

test("an application configures itself without a Thunder release", () => {
  // The whole point: a product Thunder has never heard of — including one
  // that does not exist yet — is expressible today.
  const future = Config.standard().withScheme("nobody-shipped-this-yet").withPort(4242);
  expect(future.scheme).toBe("nobody-shipped-this-yet");
  expect(future.defaultPort).toBe(4242);
  // …and it inherits every standard behavior it did not override.
  expect(future.handshake).toBe(Config.standard().handshake);
  expect(future.errorCodes).toBe(Config.standard().errorCodes);
});

test("overrides compose and leave the rest standard", () => {
  // A deployment that still diverges says so in its own repository.
  const diverging = Config.standard()
    .withScheme("legacy")
    .withPort(15501)
    .withHandshake("auth_command")
    .withHelloStyle("not_used")
    .withPush("enabled")
    .withMaxFrameBytes(512 * 1024 * 1024)
    .withErrorCodes("resp3_prefixes");

  expect(diverging.handshake).toBe("auth_command");
  expect(diverging.push).toBe("enabled");
  expect(diverging.maxFrameBytes).toBe(512 * 1024 * 1024);
  // Untouched dimensions stay standard — convergence is "delete overrides
  // until only identity remains".
  expect(diverging.maxInFlight).toBe(Config.standard().maxInFlight);
  expect(diverging.tls).toBe(Config.standard().tls);
});

test("every dimension has an override, and each returns a new config", () => {
  const standard = Config.standard();
  const overridden = standard
    .withScheme("app")
    .withPort(1)
    .withHandshake("none")
    .withHelloStyle("arg_less")
    .withPush("enabled")
    .withMaxFrameBytes(1024)
    .withMaxInFlight(2)
    .withErrorCodes("none")
    .withTls("optional");

  expect({ ...overridden }).toEqual({
    scheme: "app",
    defaultPort: 1,
    handshake: "none",
    helloStyle: "arg_less",
    push: "enabled",
    maxFrameBytes: 1024,
    maxInFlight: 2,
    errorCodes: "none",
    tls: "optional",
  });
  // The receiver is untouched: overrides never mutate (PRO-003).
  expect(standard.scheme).toBe("");
  expect(standard.maxFrameBytes).toBe(DEFAULT_MAX_FRAME_BYTES);
  expect(Object.isFrozen(standard)).toBe(true);
});

test("a config is still plain data", () => {
  // Configs are data (PRO-003): object construction must keep working, so
  // nothing forces an application through the builder.
  const literal: Config = {
    scheme: "plain",
    defaultPort: 1,
    handshake: "none",
    helloStyle: "not_used",
    push: "reserved",
    maxFrameBytes: 1024,
    maxInFlight: 2,
    errorCodes: "none",
    tls: "off",
  };
  expect(literal.scheme).toBe("plain");

  // Spreading a built config yields plain data (the overrides live on the
  // prototype), and `Config.from` lifts data back into the chainable form.
  const spread = { ...Config.standard(), scheme: "spread" };
  expect(Config.from(spread).withPort(7).scheme).toBe("spread");
  expect(Config.from(spread).withPort(7).defaultPort).toBe(7);
  expect(Config.from(literal).withScheme("lifted").maxInFlight).toBe(2);
});
