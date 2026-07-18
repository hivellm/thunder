#!/usr/bin/env python3
"""Cross-language live-interop probe (Python client vs the Rust server).

    python interop/probe.py client <port>

Speaks the family standard config (mandatory HELLO map). Prints `OK` and exits
0 on success, `FAIL: <why>` and exits 1 otherwise. The server is Rust-only
(SPEC-004), so this probe is client-only.
"""

import os
import sys

# Use the local (uncommitted) source, not any installed package.
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "python"))

from thunder_rpc import Client, ClientConfig, Config, Value, errors  # noqa: E402

PAYLOAD = "cross-language-🌩"


def fail(why: str) -> None:
    print(f"FAIL: {why}")
    sys.exit(1)


def main() -> None:
    if len(sys.argv) != 3 or sys.argv[1] != "client":
        print("usage: probe.py client <port> (server is Rust-only)", file=sys.stderr)
        sys.exit(2)
    port = int(sys.argv[2])

    config = Config.standard().with_scheme("interop").with_port(0)
    try:
        client = Client.connect(
            f"127.0.0.1:{port}", config, ClientConfig(client_name="python")
        )
    except Exception as e:  # noqa: BLE001 - a probe reports any failure
        fail(f"connect/handshake failed: {e}")

    try:
        pong = client.call("PING")
        if pong.as_str() != "PONG":
            fail(f"PING returned {pong!r}, want PONG")

        echo = client.call("ECHO", [Value.str(PAYLOAD)])
        if echo.as_str() != PAYLOAD:
            fail(f"ECHO returned {echo!r}, want {PAYLOAD!r}")

        try:
            client.call("NOPE")
            fail("NOPE returned ok, want a typed error")
        except errors.ThunderError:
            pass  # a typed error is exactly right
    finally:
        client.close()

    print("OK")
    sys.exit(0)


if __name__ == "__main__":
    main()
