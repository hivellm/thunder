/**
 * Live interop smoke (TST-050) — a Thunder client against a REAL product
 * instance. Env-gated and skipped by default. Set any of
 * THUNDER_LIVE_URL_SYNAP / _NEXUS / _VECTORIZER to a reachable endpoint
 * (e.g. `synap://host:port`) and this connects with that product's deployment
 * shape (BN-023), makes a PING-class call, one typed-error call, and closes.
 * With none set it skips and passes — not part of the always-on floor.
 */
import { test } from "vitest";

import { Client, Config } from "../src/index";

function synapShape(): Config {
  return {
    ...Config.standard().withScheme("synap").withPort(0),
    handshake: "auth_command",
    helloStyle: "not_used",
    errorCodes: "resp3_prefixes",
  };
}

function nexusShape(): Config {
  return {
    ...Config.standard().withScheme("nexus").withPort(0),
    handshake: "auth_command",
    helloStyle: "arg_less",
    errorCodes: "resp3_prefixes",
  };
}

function vectorizerShape(): Config {
  return Config.standard().withScheme("vectorizer").withPort(0);
}

const PRODUCTS: { env: string; shape: () => Config }[] = [
  { env: "THUNDER_LIVE_URL_SYNAP", shape: synapShape },
  { env: "THUNDER_LIVE_URL_NEXUS", shape: nexusShape },
  { env: "THUNDER_LIVE_URL_VECTORIZER", shape: vectorizerShape },
];

test("live interop smoke (TST-050, env-gated)", async () => {
  let ran = 0;
  for (const { env, shape } of PRODUCTS) {
    const url = process.env[env];
    if (!url) {
      console.error(`live smoke: ${env} unset — skipped (release-path only)`);
      continue;
    }
    const client = await Client.connect(url, shape(), { clientName: "thunder-live-smoke" });
    try {
      // A PING-class call must succeed.
      await client.call("PING");
      // A command no product implements must come back a typed error.
      let errored = false;
      try {
        await client.call("__thunder_live_smoke_unknown__");
      } catch {
        errored = true;
      }
      if (!errored) throw new Error(`${env}: bogus command returned ok, expected a typed error`);
    } finally {
      await client.close();
    }
    ran += 1;
  }
  if (ran === 0) {
    console.error("live smoke: no THUNDER_LIVE_URL_* set — nothing to run (expected)");
  }
});
