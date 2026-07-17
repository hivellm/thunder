"""Profile handshake semantics shared by the sync and async clients
(CLT-002/003). The exchange is expressed sans-IO as a generator so the two
clients differ in idiom only, never in semantics (FR-28):

- the generator yields ``(command, args)`` round trips,
- the driver sends back the reply :class:`~thunder_rpc.value.Value`,
- the generator returns the final :class:`~thunder_rpc.config.HandshakeInfo`.
"""

from __future__ import annotations

from typing import Generator

from . import errors
from .config import ClientConfig, Credentials, HandshakeInfo
from .profile import Handshake, HelloStyle, Profile
from .value import Value

#: Reconnect backoff: first re-dial retries after BACKOFF_BASE seconds,
#: doubling up to BACKOFF_CAP (CLT-030 "capped backoff").
BACKOFF_BASE = 0.05
BACKOFF_CAP = 0.5

#: Re-dial budget when a call finds the connection dead (CLT-030).
RECONNECT_ATTEMPTS = 2

#: Client name announced in the HELLO map when none is configured.
DEFAULT_CLIENT_NAME = "thunder-client"

Exchange = Generator["tuple[str, tuple[Value, ...]]", Value, HandshakeInfo]


def handshake_exchange(profile: Profile, config: ClientConfig) -> Exchange:
    """Run the profile handshake before user calls proceed (CLT-002):
    ``NONE`` sends nothing; ``AUTH_COMMAND`` sends optional ``HELLO`` then
    ``AUTH`` when credentials are configured; ``HELLO_MANDATORY`` sends the
    ``HELLO`` map as the first frame and parses the reply.
    """
    if profile.handshake is Handshake.NONE:
        return HandshakeInfo()
    if profile.handshake is Handshake.AUTH_COMMAND:
        credentials = config.credentials
        if credentials is None:
            return HandshakeInfo()
        if profile.hello_style is HelloStyle.POSITIONAL_VERSION:
            # Optional HELLO announcing the protocol version.
            yield ("HELLO", (Value.int(1),))
        yield ("AUTH", tuple(Value.str(secret) for secret in credentials.secrets))
        return HandshakeInfo(authenticated=True)
    # HELLO_MANDATORY: the HELLO map is the first frame (PRO-001). Pair
    # order (version, credential, client_name) is corpus-pinned.
    pairs = [(Value.str("version"), Value.int(1))]
    credentials = config.credentials
    if credentials is not None:
        if credentials.kind == Credentials.USER_PASS:
            raise errors.AuthError(
                "user/password credentials are not supported by HelloMandatory profiles - "
                "use a token or api_key (PRO-001)"
            )
        key = "token" if credentials.kind == Credentials.TOKEN else "api_key"
        pairs.append((Value.str(key), Value.str(credentials.secrets[0])))
    name = config.client_name if config.client_name is not None else DEFAULT_CLIENT_NAME
    pairs.append((Value.str("client_name"), Value.str(name)))
    reply = yield ("HELLO", (Value.map(pairs),))
    return parse_hello_info(reply)


def parse_hello_info(reply: Value) -> HandshakeInfo:
    """Extract ``authenticated`` / ``capabilities`` from a HELLO reply map."""
    authenticated = False
    node = reply.map_get("authenticated")
    if node is not None and node.as_bool() is not None:
        authenticated = node.as_bool()
    capabilities: tuple[str, ...] = ()
    node = reply.map_get("capabilities")
    if node is not None:
        items = node.as_array()
        if items is not None:
            capabilities = tuple(v.as_str() for v in items if v.as_str() is not None)
    return HandshakeInfo(authenticated=authenticated, capabilities=capabilities)


def as_handshake_error(error: errors.ThunderError) -> errors.ThunderError:
    """Server rejections during the handshake surface as the typed auth
    class, never a generic error (CLT-003); transport failures keep their
    own class."""
    if isinstance(error, errors.AuthError):
        return error
    if isinstance(error, errors.ServerError):
        return errors.AuthError(error.message)
    return error
