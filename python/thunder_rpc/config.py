"""Protocol configuration (SPEC-002) — the declarative description of how
**one application** uses the shared wire. Pure data: the codec never depends
on it; the clients drive their behavior from it.

Thunder ships one standard and zero product knowledge
-----------------------------------------------------

There are no named configurations here — no per-product constructors, no
registry of products. Thunder was born from three products' RPC
implementations, but a protocol library that must serve implementations
which do not exist yet cannot ship a hardcoded list of the ones that did.

Instead: :meth:`Config.standard` is **the** family standard, and every
dimension is a knob. An application that matches the standard writes its
identity and nothing else::

    from thunder_rpc import Config

    config = Config.standard().with_scheme("myapp").with_port(9000)

An application that still diverges says so **in its own repository**, where
that knowledge belongs::

    from thunder_rpc import Config, Handshake, HelloStyle, PushPolicy

    # A deployment whose RPC path authenticates via AUTH and has no HELLO
    # handler, and which ships a push-producing command.
    config = (
        Config.standard()
        .with_scheme("legacy")
        .with_port(15501)
        .with_handshake(Handshake.AUTH_COMMAND)
        .with_hello_style(HelloStyle.NOT_USED)
        .with_push(PushPolicy.ENABLED)
    )

Convergence is therefore visible and per-application: delete overrides until
only ``scheme`` and ``port`` remain. Nobody waits on a Thunder release for a
row in a registry, and Thunder never carries behavior it does not own.

The standard's values are pinned to ``conformance/standard.yaml`` by a test
in every language, so the four implementations can never disagree about what
"standard" means — the one guarantee the old per-product registry
legitimately provided.

The builder methods are named ``with_*`` because :class:`Config` is a frozen
dataclass: a method named ``scheme`` would shadow the ``scheme`` field on the
class and make ``config.scheme`` unreadable. ``with_*`` keeps both the data
access and the chainable override, each returning a NEW config
(:func:`dataclasses.replace` semantics); plain ``Config(...)`` construction
stays supported and neither form requires a Thunder release.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from enum import Enum

from .value import DEFAULT_MAX_FRAME_BYTES


class Handshake(Enum):
    """Handshake style (PRO-001)."""

    #: No RPC-layer handshake at all: the connection is usable immediately.
    NONE = "none"
    #: ``HELLO`` optional; ``AUTH [api_key]`` / ``[user, pass]`` /
    #: ``[password]``; pre-auth allowlist ``PING/HELLO/AUTH/QUIT``.
    #:
    #: Whether a deployment *enforces* credentials is its own config, not a
    #: protocol dialect: a client with no credentials configured simply sends
    #: no ``AUTH``, which is correct against an open deployment (PRO-001a).
    AUTH_COMMAND = "auth_command"
    #: ``HELLO`` must be the first frame, carrying credentials. **The
    #: standard** — see :meth:`Config.standard`.
    HELLO_MANDATORY = "hello_mandatory"


class HelloStyle(Enum):
    """HELLO payload style (PRO-001)."""

    #: The application has no ``HELLO`` command.
    NOT_USED = "not_used"
    #: ``HELLO`` with **no arguments**; the reply is a metadata Map
    #: ``{server, version, proto, id, authenticated}``. Credentials travel via
    #: ``AUTH``, never inside the HELLO.
    ARG_LESS = "arg_less"
    #: Map with ``version``, ``token`` | ``api_key``, ``client_name``; the
    #: reply carries ``proto`` and ``capabilities``. **The standard** — the
    #: only style that negotiates a version and advertises capabilities, which
    #: is what an evolving protocol needs.
    MAP_PAYLOAD = "map_payload"


class PushPolicy(Enum):
    """Server-push policy (PRO-001)."""

    #: ``PUSH_ID`` reserved: servers refuse it from clients and never emit it.
    #: **The standard** — emitting push is a capability an application opts
    #: into by shipping a push-producing command.
    RESERVED = "reserved"
    #: Push frames flow to the client's push hook.
    ENABLED = "enabled"


class ErrorConvention(Enum):
    """Which error-string prefix conventions the client parses (PRO-014)."""

    #: No prefix parsing.
    NONE = "none"
    #: ``ERR`` / ``NOAUTH`` / ``WRONGPASS`` / ``NOPERM`` prefixes.
    RESP3_PREFIXES = "resp3_prefixes"
    #: Leading ``"[<code>] "`` machine-readable code.
    BRACKET_CODE = "bracket_code"
    #: Both conventions composed. **The standard** — a strict superset, so it
    #: parses either grammar and needs no negotiation.
    BOTH = "both"


class TlsPolicy(Enum):
    """Transport-security policy (PRO-001)."""

    #: Plain TCP. **The standard default** — TLS is an additive capability a
    #: deployment turns on, never a dialect.
    OFF = "off"
    #: TLS available behind configuration.
    OPTIONAL = "optional_rustls"
    #: Config keys reserved; not wired yet.
    RESERVED = "reserved_config"


@dataclass(frozen=True)
class Config:
    """One application's protocol configuration (PRO-001).

    Configs are **data, never behavior**: no config may alter wire bytes
    (PRO-003) — it selects among behaviors Thunder already implements.
    Construct with :meth:`standard` and the ``with_*`` builder, or as a plain
    dataclass; both are supported and neither requires a Thunder release.

    The field defaults *are* the standard, so ``Config()`` equals
    ``Config.standard()``.
    """

    #: URL scheme the endpoint parser accepts for this application (PRO-012).
    #: Identity — Thunder has no default for it.
    scheme: str = ""
    #: Default RPC port for the scheme (PRO-012). Identity — Thunder has no
    #: default for it.
    default_port: int = 0
    #: Handshake style.
    handshake: Handshake = Handshake.HELLO_MANDATORY
    #: HELLO payload style.
    hello_style: HelloStyle = HelloStyle.MAP_PAYLOAD
    #: Server-push policy.
    push: PushPolicy = PushPolicy.RESERVED
    #: Frame cap (WIRE-020).
    max_frame_bytes: int = DEFAULT_MAX_FRAME_BYTES
    #: Per-connection in-flight request bound (CLT-012 / SRV-003).
    max_in_flight: int = 256
    #: Error-string conventions the client parses.
    error_codes: ErrorConvention = ErrorConvention.BOTH
    #: Transport-security policy.
    tls: TlsPolicy = TlsPolicy.OFF

    @classmethod
    def standard(cls) -> Config:
        """**The** family standard (pinned by ``conformance/standard.yaml``).

        Mandatory ``HELLO`` map with ``proto`` negotiation and a capabilities
        reply; the ``[CODE]`` error superset; 64 MiB frames; 256 in-flight;
        push reserved; TLS off.

        ``scheme`` is ``""`` and ``default_port`` is ``0`` — identity is the
        application's to supply, and a :class:`Config` that never sets them is
        only usable with an explicit ``host:port`` endpoint.
        """
        return cls()

    def with_scheme(self, scheme: str) -> Config:
        """The URL scheme this application answers on (PRO-012)."""
        return replace(self, scheme=scheme)

    def with_port(self, port: int) -> Config:
        """The default RPC port for the scheme (PRO-012)."""
        return replace(self, default_port=port)

    def with_handshake(self, handshake: Handshake) -> Config:
        """Override the handshake style."""
        return replace(self, handshake=handshake)

    def with_hello_style(self, hello_style: HelloStyle) -> Config:
        """Override the HELLO payload style."""
        return replace(self, hello_style=hello_style)

    def with_push(self, push: PushPolicy) -> Config:
        """Override the server-push policy."""
        return replace(self, push=push)

    def with_max_frame_bytes(self, max_frame_bytes: int) -> Config:
        """Override the frame cap (WIRE-020)."""
        return replace(self, max_frame_bytes=max_frame_bytes)

    def with_max_in_flight(self, max_in_flight: int) -> Config:
        """Override the per-connection in-flight bound."""
        return replace(self, max_in_flight=max_in_flight)

    def with_error_codes(self, error_codes: ErrorConvention) -> Config:
        """Override the error-string conventions parsed."""
        return replace(self, error_codes=error_codes)

    def with_tls(self, tls: TlsPolicy) -> Config:
        """Override the transport-security policy."""
        return replace(self, tls=tls)
