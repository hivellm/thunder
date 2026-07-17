"""Protocol profiles (SPEC-002) — the declarative description of how one
product uses the shared wire. Pure data: the codec never depends on it; the
clients drive their behavior from it.

The family registry constants below are generated-by-hand from
``conformance/profiles/*.yaml`` (PRO-010) and pinned to those files by
``tests/test_profiles.py`` — server and SDKs of one product can never
disagree. Custom construction stays public (PRO-020): new products never
wait for a Thunder release.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum

from .value import DEFAULT_MAX_FRAME_BYTES


class Handshake(Enum):
    """Handshake style (PRO-001)."""

    #: No RPC-layer auth (Synap v1 legacy).
    NONE = "none"
    #: ``HELLO`` optional; ``AUTH [api_key]`` or ``[user, pass]``; pre-auth
    #: allowlist ``PING/HELLO/AUTH/QUIT`` (Nexus).
    AUTH_COMMAND = "auth_command"
    #: ``HELLO`` must be the first frame, carrying credentials
    #: (Vectorizer / Lexum).
    HELLO_MANDATORY = "hello_mandatory"


class HelloStyle(Enum):
    """HELLO payload style (PRO-001)."""

    #: No HELLO in the profile (Synap).
    NOT_USED = "not_used"
    #: Positional ``[Int(version)]`` (Nexus).
    POSITIONAL_VERSION = "positional_version"
    #: Map with ``version``, ``token`` | ``api_key``, ``client_name``; reply
    #: carries ``capabilities`` (Vectorizer / Lexum).
    MAP_PAYLOAD = "map_payload"


class PushPolicy(Enum):
    """Server-push policy (PRO-001)."""

    #: ``PUSH_ID`` reserved: servers refuse it from clients and never emit it.
    RESERVED = "reserved"
    #: Push frames flow (Synap ``SUBSCRIBE``).
    ENABLED = "enabled"


class ErrorConvention(Enum):
    """Which error-string prefix conventions the client parses (PRO-014)."""

    #: No prefix parsing.
    NONE = "none"
    #: ``ERR`` / ``NOAUTH`` / ``WRONGPASS`` / ``NOPERM`` prefixes (Nexus, Synap).
    RESP3_PREFIXES = "resp3_prefixes"
    #: Leading ``"[<code>] "`` machine-readable code (Vectorizer).
    BRACKET_CODE = "bracket_code"
    #: Both conventions composed (Lexum).
    BOTH = "both"


class TlsPolicy(Enum):
    """Transport-security policy (PRO-001)."""

    #: Plain TCP.
    OFF = "off"
    #: TLS available behind configuration.
    OPTIONAL = "optional_rustls"
    #: Config keys reserved; not wired yet.
    RESERVED = "reserved_config"


@dataclass(frozen=True)
class Profile:
    """One product's protocol profile (PRO-001). Profiles are data, never
    behavior: no profile may alter wire bytes (PRO-003). Fields beyond the
    identity triple carry defaults so adding a field stays a minor release
    (PRO-002).
    """

    #: Registry name (``synap``, ``nexus``, ...) or a custom identifier.
    name: str
    #: URL scheme the endpoint parser registers for this profile (PRO-012).
    scheme: str
    #: Default RPC port for the scheme (PRO-012).
    default_port: int
    handshake: Handshake = Handshake.NONE
    hello_style: HelloStyle = HelloStyle.NOT_USED
    push: PushPolicy = PushPolicy.RESERVED
    #: Frame cap (WIRE-020).
    max_frame_bytes: int = DEFAULT_MAX_FRAME_BYTES
    #: Per-connection in-flight request bound (CLT-012).
    max_in_flight: int = 256
    error_codes: ErrorConvention = ErrorConvention.NONE
    tls: TlsPolicy = TlsPolicy.OFF


#: Synap — protocol origin. No RPC-layer auth, push enabled, 512 MiB cap
#: (matches ``synap-protocol``'s ``MAX_FRAME_SIZE``).
SYNAP = Profile(
    name="synap",
    scheme="synap",
    default_port=15501,
    handshake=Handshake.NONE,
    hello_style=HelloStyle.NOT_USED,
    push=PushPolicy.ENABLED,
    max_frame_bytes=512 * 1024 * 1024,
    max_in_flight=256,
    error_codes=ErrorConvention.RESP3_PREFIXES,
    tls=TlsPolicy.OFF,
)

#: Nexus — canonical spec author. Optional HELLO + AUTH, 64 MiB cap.
NEXUS = Profile(
    name="nexus",
    scheme="nexus",
    default_port=15475,
    handshake=Handshake.AUTH_COMMAND,
    hello_style=HelloStyle.POSITIONAL_VERSION,
    push=PushPolicy.RESERVED,
    max_frame_bytes=DEFAULT_MAX_FRAME_BYTES,
    max_in_flight=1024,
    error_codes=ErrorConvention.RESP3_PREFIXES,
    tls=TlsPolicy.OFF,
)

#: Vectorizer — HELLO-mandatory with credentials, ``[code]`` prefixes.
VECTORIZER = Profile(
    name="vectorizer",
    scheme="vectorizer",
    default_port=15503,
    handshake=Handshake.HELLO_MANDATORY,
    hello_style=HelloStyle.MAP_PAYLOAD,
    push=PushPolicy.RESERVED,
    max_frame_bytes=DEFAULT_MAX_FRAME_BYTES,
    max_in_flight=256,
    error_codes=ErrorConvention.BRACKET_CODE,
    tls=TlsPolicy.OPTIONAL,
)

#: Lexum — Vectorizer-style handshake, both error conventions.
LEXUM = Profile(
    name="lexum",
    scheme="lexum",
    default_port=17001,
    handshake=Handshake.HELLO_MANDATORY,
    hello_style=HelloStyle.MAP_PAYLOAD,
    push=PushPolicy.RESERVED,
    max_frame_bytes=DEFAULT_MAX_FRAME_BYTES,
    max_in_flight=256,
    error_codes=ErrorConvention.BOTH,
    tls=TlsPolicy.RESERVED,
)


def registry() -> tuple[Profile, ...]:
    """Every registered family profile (PRO-010)."""
    return (SYNAP, NEXUS, VECTORIZER, LEXUM)


class Profiles:
    """The family registry as attributes (PRO-010: ``Profiles.synap`` etc.)."""

    synap = SYNAP
    nexus = NEXUS
    vectorizer = VECTORIZER
    lexum = LEXUM
