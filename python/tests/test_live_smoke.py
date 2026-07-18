"""Live interop smoke (TST-050) — the Python client against a REAL product.

Env-gated and skipped by default. Set any of THUNDER_LIVE_URL_SYNAP / _NEXUS /
_VECTORIZER to a reachable endpoint (e.g. ``synap://host:port``) and this
connects with that product's deployment shape (BN-023), makes a PING-class
call, one typed-error call, and closes. With none set it skips and passes — not
part of the always-on floor.
"""

from __future__ import annotations

import os

from thunder_rpc import (
    Client,
    ClientConfig,
    Config,
    ErrorConvention,
    Handshake,
    HelloStyle,
    errors,
)


def _synap() -> Config:
    return (
        Config.standard()
        .with_scheme("synap")
        .with_handshake(Handshake.AUTH_COMMAND)
        .with_hello_style(HelloStyle.NOT_USED)
        .with_error_codes(ErrorConvention.RESP3_PREFIXES)
    )


def _nexus() -> Config:
    return (
        Config.standard()
        .with_scheme("nexus")
        .with_handshake(Handshake.AUTH_COMMAND)
        .with_hello_style(HelloStyle.ARG_LESS)
        .with_error_codes(ErrorConvention.RESP3_PREFIXES)
    )


def _vectorizer() -> Config:
    return Config.standard().with_scheme("vectorizer")


_PRODUCTS = [
    ("THUNDER_LIVE_URL_SYNAP", _synap),
    ("THUNDER_LIVE_URL_NEXUS", _nexus),
    ("THUNDER_LIVE_URL_VECTORIZER", _vectorizer),
]


def test_live_interop_smoke() -> None:
    ran = 0
    for env, shape in _PRODUCTS:
        url = os.environ.get(env)
        if not url:
            print(f"live smoke: {env} unset — skipped (release-path only)")
            continue
        client = Client.connect(
            url, shape(), ClientConfig(client_name="thunder-live-smoke")
        )
        try:
            # A PING-class call must succeed.
            client.call("PING")
            # A command no product implements must come back a typed error.
            errored = False
            try:
                client.call("__thunder_live_smoke_unknown__")
            except errors.ThunderError:
                errored = True
            assert errored, f"{env}: bogus command returned ok, expected a typed error"
        finally:
            client.close()
        ran += 1
    if ran == 0:
        print("live smoke: no THUNDER_LIVE_URL_* set — nothing to run (expected)")
