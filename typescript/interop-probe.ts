/**
 * Cross-language live-interop probe (TypeScript client vs the Rust server).
 *
 *   npx tsx interop-probe.ts client <port>
 *
 * Speaks the family standard config (mandatory HELLO map). Prints `OK` + exit 0
 * on success, `FAIL: <why>` + exit 1 otherwise. The server is Rust-only
 * (SPEC-004), so this probe is client-only. Imports the local source.
 */
import { Client, Config, Value } from "./src/index";

const PAYLOAD = "cross-language-🌩";

function fail(why: string): never {
  console.log(`FAIL: ${why}`);
  process.exit(1);
}

async function main(): Promise<void> {
  const [role, portStr] = process.argv.slice(2);
  if (role !== "client") {
    console.error("usage: interop-probe.ts client <port> (server is Rust-only)");
    process.exit(2);
  }
  const port = Number(portStr);
  const config = Config.standard().withScheme("interop").withPort(0);

  let client: Client;
  try {
    client = await Client.connect(`127.0.0.1:${port}`, config, { clientName: "typescript" });
  } catch (e) {
    fail(`connect/handshake failed: ${e}`);
  }

  try {
    const pong = await client.call("PING");
    if (Value.asStr(pong) !== "PONG") fail(`PING returned ${JSON.stringify(pong)}, want PONG`);

    const echo = await client.call("ECHO", [Value.str(PAYLOAD)]);
    if (Value.asStr(echo) !== PAYLOAD) fail(`ECHO returned ${JSON.stringify(echo)}, want ${PAYLOAD}`);

    let errored = false;
    try {
      await client.call("NOPE");
    } catch {
      errored = true; // a typed error is exactly right
    }
    if (!errored) fail("NOPE returned ok, want a typed error");
  } finally {
    await client.close();
  }

  console.log("OK");
  process.exit(0);
}

void main();
