"""Endpoint parsing (CLT-070/071).

Accepts ``scheme://host[:port]`` where the scheme is **the application's
own**, taken from its :class:`~thunder_rpc.config.Config` — Thunder has no
registry of schemes to consult and no product's parser to fork (PRO-012) —
plus bare ``host:port`` (RPC implied). ``http(s)://`` URLs are rejected with
a pointer to the application's HTTP client: Thunder is RPC-only.

Parse failures use the :class:`~thunder_rpc.errors.ConnectionError` class —
an endpoint that cannot be parsed is an endpoint that cannot be dialed.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

from . import errors
from .config import Config


@dataclass(frozen=True)
class Endpoint:
    """A resolved RPC endpoint: host plus concrete port."""

    #: Host name or IP literal (IPv6 without brackets).
    host: str
    #: Concrete port — explicit, or the config's ``default_port``.
    port: int


def parse_endpoint(text: str, config: Config) -> Endpoint:
    """Parse an endpoint string against the application's
    :class:`~thunder_rpc.config.Config` (CLT-070).

    Accepted forms:

    - ``scheme://host[:port]`` where ``scheme`` is ``config.scheme``; a
      missing port resolves to ``config.default_port`` (CLT-071).
    - bare ``host:port`` (RPC implied).
    - ``[v6::addr]:port`` / ``scheme://[v6::addr][:port]`` for IPv6 literals.

    ``http://`` / ``https://`` are rejected: Thunder is RPC-only; REST
    endpoints belong to the application's HTTP client.
    """
    text = text.strip()
    if "://" in text:
        scheme, rest = text.split("://", 1)
        scheme = scheme.lower()
        if scheme in ("http", "https"):
            raise _invalid(
                f"'{text}' is an HTTP URL and Thunder is RPC-only - use the application's "
                "HTTP client for REST endpoints, or pass an RPC endpoint such as "
                "'scheme://host:port' or bare 'host:port'"
            )
        if scheme != config.scheme:
            raise _invalid(
                f"endpoint scheme '{scheme}' does not match this client's configured "
                f"scheme '{config.scheme}' - set the scheme on the Config, or use bare "
                "'host:port'"
            )
        if rest.endswith("/"):
            rest = rest[:-1]
        if "/" in rest:
            raise _invalid(
                f"endpoint '{text}' must not carry a path - expected {scheme}://host[:port]"
            )
        host, port = _split_host_port(rest)
        return Endpoint(host, port if port is not None else config.default_port)
    host, port = _split_host_port(text)
    if port is None:
        raise _invalid(
            f"bare endpoint '{text}' needs an explicit port ('host:port') - only "
            "scheme-prefixed endpoints resolve a registry default port"
        )
    return Endpoint(host, port)


def _split_host_port(s: str) -> tuple[str, int | None]:
    """Split ``host[:port]``, handling bracketed IPv6 literals."""
    if not s:
        raise _invalid("endpoint host is empty")
    if s.startswith("["):
        inner = s[1:]
        if "]" not in inner:
            raise _invalid(f"unterminated '[' in endpoint host '{s}'")
        host, tail = inner.split("]", 1)
        if not host:
            raise _invalid("endpoint host is empty")
        if tail == "":
            return host, None
        if not tail.startswith(":"):
            raise _invalid(f"expected ':port' after ']' in endpoint '{s}'")
        return host, _parse_port(tail[1:], s)
    head, sep, port = s.rpartition(":")
    if not sep:
        return s, None
    if ":" in head:
        # More than one ':' without brackets: an IPv6 literal, no port.
        return s, None
    if not head:
        raise _invalid("endpoint host is empty")
    return head, _parse_port(port, s)


def _parse_port(port: str, whole: str) -> int:
    if re.fullmatch(r"[0-9]+", port):
        value = int(port)
        if value <= 65535:
            return value
    raise _invalid(f"invalid port '{port}' in endpoint '{whole}'")


def _invalid(message: str) -> errors.ConnectionError:
    return errors.ConnectionError(message)
