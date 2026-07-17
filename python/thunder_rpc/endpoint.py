"""Endpoint parsing (CLT-070/071).

Accepts ``scheme://host[:port]`` for every scheme in the profile registry —
scheme → default-port resolution is data-driven (PRO-012), products never
fork the parser — plus bare ``host:port`` (RPC implied; the caller supplies
the profile). ``http(s)://`` URLs are rejected with a pointer to the
product's HTTP client: Thunder is RPC-only.

Parse failures use the :class:`~thunder_rpc.errors.ConnectionError` class —
an endpoint that cannot be parsed is an endpoint that cannot be dialed.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

from . import errors
from .profile import registry


@dataclass(frozen=True)
class Endpoint:
    """A resolved RPC endpoint: host plus concrete port."""

    #: Host name or IP literal (IPv6 without brackets).
    host: str
    #: Concrete port — explicit, or the scheme's registry default.
    port: int


def parse_endpoint(text: str) -> Endpoint:
    """Parse an endpoint string (CLT-070).

    Accepted forms:

    - ``scheme://host[:port]`` for every registered profile scheme
      (``synap``, ``nexus``, ``vectorizer``, ``lexum``); a missing port
      resolves to the scheme's registry default (CLT-071).
    - bare ``host:port`` (RPC implied — the caller supplies the profile).
    - ``[v6::addr]:port`` / ``scheme://[v6::addr][:port]`` for IPv6 literals.

    ``http://`` / ``https://`` are rejected: Thunder is RPC-only; REST
    endpoints belong to the product's HTTP client.
    """
    text = text.strip()
    if "://" in text:
        scheme, rest = text.split("://", 1)
        scheme = scheme.lower()
        if scheme in ("http", "https"):
            raise _invalid(
                f"'{text}' is an HTTP URL and Thunder is RPC-only - use the product's HTTP "
                "client for REST endpoints, or pass an RPC endpoint such as "
                "'vectorizer://host:port' or bare 'host:port'"
            )
        profile = next((p for p in registry() if p.scheme == scheme), None)
        if profile is None:
            known = ", ".join(p.scheme for p in registry())
            raise _invalid(
                f"unknown endpoint scheme '{scheme}' - registered schemes: {known}; "
                "or use bare 'host:port'"
            )
        if rest.endswith("/"):
            rest = rest[:-1]
        if "/" in rest:
            raise _invalid(
                f"endpoint '{text}' must not carry a path - expected {scheme}://host[:port]"
            )
        host, port = _split_host_port(rest)
        return Endpoint(host, port if port is not None else profile.default_port)
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
