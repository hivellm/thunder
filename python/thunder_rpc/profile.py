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

    #: No RPC-layer handshake at all: the connection is usable immediately.
    #:
    #: No registered family profile uses this. It was the mistaken reading of
    #: Synap, whose RPC path *does* authenticate (``AUTH`` handler behind its
    #: ``require_auth`` toggle) — see the BN-023 errata. It stays available
    #: for custom profiles (PRO-020).
    NONE = "none"
    #: ``HELLO`` optional; ``AUTH [api_key]`` or ``[user, pass]``; pre-auth
    #: allowlist ``PING/HELLO/AUTH/QUIT`` (Nexus, Synap).
    #:
    #: Whether a deployment *enforces* credentials is its own config
    #: (``auth_required`` / ``require_auth``), not a protocol dialect: a client
    #: with no credentials configured simply sends no ``AUTH``, which is
    #: correct against an open deployment.
    AUTH_COMMAND = "auth_command"
    #: ``HELLO`` must be the first frame, carrying credentials
    #: (Vectorizer / Lexum).
    HELLO_MANDATORY = "hello_mandatory"


class HelloStyle(Enum):
    """HELLO payload style (PRO-001)."""

    #: The profile has no ``HELLO`` command (Synap: its RPC path ships an
    #: ``AUTH`` handler but no ``HELLO`` handler at all).
    NOT_USED = "not_used"
    #: ``HELLO`` with **no arguments**; the reply is a metadata Map
    #: ``{server, version, proto, id, authenticated}`` (Nexus). Credentials
    #: travel via ``AUTH``, never inside the HELLO.
    ARG_LESS = "arg_less"
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


#: Synap — protocol origin. ``AUTH``-command auth with **no HELLO**, push
#: enabled, 512 MiB cap (matches ``synap-protocol``'s ``MAX_FRAME_SIZE``).
#:
#: Its RPC listener authenticates inline in the read loop (``AUTH`` → shared
#: ``UserManager``, ``NOAUTH`` gate, ``NOPERM`` admin ACL) behind the
#: ``require_auth`` config toggle; it simply has no ``HELLO`` handler. The
#: registry previously said ``handshake: none``, which described only the
#: ``require_auth = false`` posture and left this profile unable to
#: authenticate at all (BN-023 errata).
SYNAP = Profile(
    name="synap",
    scheme="synap",
    default_port=15501,
    handshake=Handshake.AUTH_COMMAND,
    hello_style=HelloStyle.NOT_USED,
    push=PushPolicy.ENABLED,
    max_frame_bytes=512 * 1024 * 1024,
    max_in_flight=256,
    error_codes=ErrorConvention.RESP3_PREFIXES,
    tls=TlsPolicy.OFF,
)

#: Nexus — canonical spec author. Optional arg-less HELLO + AUTH, 64 MiB cap.
#:
#: Its RPC ``HELLO`` takes no arguments and answers with a metadata Map; the
#: positional ``[Int(1)]`` the registry used to claim is the *RESP3* HELLO, a
#: different surface (BN-023 errata).
NEXUS = Profile(
    name="nexus",
    scheme="nexus",
    default_port=15475,
    handshake=Handshake.AUTH_COMMAND,
    hello_style=HelloStyle.ARG_LESS,
    push=PushPolicy.RESERVED,
    max_frame_bytes=DEFAULT_MAX_FRAME_BYTES,
    max_in_flight=1024,
    error_codes=ErrorConvention.RESP3_PREFIXES,
    tls=TlsPolicy.OFF,
)

#: Vectorizer — HELLO-mandatory with credentials, ``[code]`` prefixes.
#:
#: TLS is described in its RPC spec but never wired — its ``RpcConfig`` exposes
#: no cert/key keys and the listener binds plain TCP — so the profile records
#: the capability as reserved, not optional (BN-023 errata). No family product
#: runs RPC TLS today.
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
    tls=TlsPolicy.RESERVED,
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
