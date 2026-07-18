"""Optional client TLS transport (SPEC-008 CAN-020, FR-29).

TLS is an **additive, off-by-default** capability: the plaintext path is
untouched and pays nothing unless a :class:`ClientTls` is set on the
:class:`~thunder_rpc.client_config.ClientConfig`. There is **no STARTTLS** —
TLS is decided at connect time, before any Thunder frame is exchanged, so the
wire codec never sees the difference between a plaintext and an encrypted byte
stream.

:class:`ClientTls` is plain data (server_name + ca_path), mirroring the Rust
reference. When present, the client wraps its transport in an
:class:`ssl.SSLContext` and completes the TLS handshake before the first frame;
a setup, handshake, or verification failure classifies as the
:class:`~thunder_rpc.errors.ConnectionError` class, exactly like a plaintext
connect failure.
"""

from __future__ import annotations

import ssl
from dataclasses import dataclass


@dataclass(frozen=True)
class ClientTls:
    """Client-side TLS material (FR-29). Presence of this on the client
    config makes the client dial TLS; absence keeps it plaintext.
    """

    #: Name to verify the server certificate against (SNI). When ``None``,
    #: the endpoint host is used.
    server_name: str | None = None
    #: Path to a PEM file of trusted root(s) to pin. When ``None``, the
    #: platform's native root store is used.
    ca_path: str | None = None


def build_client_context(tls: ClientTls) -> ssl.SSLContext:
    """Build the client's verifying :class:`ssl.SSLContext` (FR-29): pin the
    configured CA, or fall back to the platform's native root store.

    ``PROTOCOL_TLS_CLIENT`` turns certificate verification and hostname
    checking on by default; we keep both on (no ``verify_mode`` / hostname
    downgrade). Raises :class:`ssl.SSLError` / :class:`OSError` on a bad CA
    file — the caller maps that onto the Connection error class.
    """
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    if tls.ca_path is not None:
        ctx.load_verify_locations(cafile=tls.ca_path)
    else:
        ctx.load_default_certs()
    return ctx


def server_name_for(tls: ClientTls, host: str) -> str:
    """The SNI / verification name: the configured ``server_name``, else the
    endpoint ``host``."""
    return tls.server_name if tls.server_name is not None else host
