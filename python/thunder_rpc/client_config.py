"""Client configuration shared by the sync and async clients (SPEC-003).

Distinct from :class:`~thunder_rpc.config.Config`, which describes the
*protocol* one application speaks (SPEC-002): this is the per-client dialing
policy — timeouts, credentials, client name.
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class Credentials:
    """Credentials for the handshake (CLT-002). Auth state is
    per-connection and sticky — there are no per-call credentials (CLT-003).

    Build via the factories: :meth:`token`, :meth:`api_key`,
    :meth:`user_pass`.
    """

    #: ``"token"`` | ``"api_key"`` | ``"user_pass"``.
    kind: str
    #: The secret parts: ``(token,)``, ``(api_key,)`` or ``(user, password)``.
    secrets: tuple[str, ...]

    TOKEN = "token"
    API_KEY = "api_key"
    USER_PASS = "user_pass"

    @classmethod
    def token(cls, token: str) -> Credentials:
        """Bearer token (``token`` key under ``HELLO_MANDATORY``)."""
        return cls(cls.TOKEN, (token,))

    @classmethod
    def api_key(cls, api_key: str) -> Credentials:
        """API key (``api_key`` key under ``HELLO_MANDATORY``, single-arg
        ``AUTH`` under ``AUTH_COMMAND``)."""
        return cls(cls.API_KEY, (api_key,))

    @classmethod
    def user_pass(cls, user: str, password: str) -> Credentials:
        """User + password (``AUTH [user, pass]`` under ``AUTH_COMMAND``)."""
        return cls(cls.USER_PASS, (user, password))


@dataclass(frozen=True)
class ClientConfig:
    """Client configuration: connect timeout default **10 s** (CLT-001),
    per-call timeout default **30 s** (CLT-020), optional credentials and
    client name for the handshake (CLT-002).
    """

    #: TCP connect timeout in seconds (CLT-001).
    connect_timeout: float = 10.0
    #: Default per-call timeout in seconds (CLT-020); override per call with
    #: ``call(..., timeout=...)``.
    call_timeout: float = 30.0
    #: Handshake credentials, when the protocol config wants them.
    credentials: Credentials | None = None
    #: Client identifier sent in the ``HELLO`` map (``HELLO_MANDATORY``).
    client_name: str | None = None


@dataclass(frozen=True)
class HandshakeInfo:
    """What the handshake learned about this connection (CLT-002)."""

    #: ``True`` once the server accepted the credentials (``AUTH`` succeeded
    #: or the ``HELLO`` reply said so).
    authenticated: bool = False
    #: Capability names from the ``HELLO`` reply (``HELLO_MANDATORY``).
    capabilities: tuple[str, ...] = ()
