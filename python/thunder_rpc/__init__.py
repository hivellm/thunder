"""hivellm-thunder — HiveLLM binary RPC (wire v1, frozen).

One frame is ``u32 LE length`` + MessagePack body; the body is a
:class:`Request` or :class:`Response` in the family's externally-tagged
encoding over the 8-variant :class:`Value` model (SPEC-001). Profiles
(SPEC-002) describe how each family product uses the shared wire; the
:class:`Client` (threads) and :class:`AsyncClient` (asyncio) implement the
uniform client contract (SPEC-003) on top.

Quickstart::

    from thunder_rpc import Client, Value, Profiles

    with Client.connect("nexus://localhost", Profiles.nexus) as client:
        pong = client.call("PING")
        assert pong.as_str() == "PONG"
"""

from . import wire
from .aio import AsyncClient
from .client import Client
from .config import ClientConfig, Credentials, HandshakeInfo
from .endpoint import Endpoint, parse_endpoint
from .errors import (
    AuthError,
    ConnectionError,
    DecodeError,
    FrameTooLargeError,
    ServerError,
    ThunderError,
    TimeoutError,
    from_server_message,
)
from .profile import (
    LEXUM,
    NEXUS,
    SYNAP,
    VECTORIZER,
    ErrorConvention,
    Handshake,
    HelloStyle,
    Profile,
    Profiles,
    PushPolicy,
    TlsPolicy,
    registry,
)
from .value import (
    DEFAULT_MAX_FRAME_BYTES,
    PUSH_ID,
    WIRE_VERSION,
    Request,
    Response,
    Value,
)

__version__ = "0.1.0"

__all__ = [
    "AsyncClient",
    "AuthError",
    "Client",
    "ClientConfig",
    "ConnectionError",
    "Credentials",
    "DEFAULT_MAX_FRAME_BYTES",
    "DecodeError",
    "Endpoint",
    "ErrorConvention",
    "FrameTooLargeError",
    "Handshake",
    "HandshakeInfo",
    "HelloStyle",
    "LEXUM",
    "NEXUS",
    "PUSH_ID",
    "Profile",
    "Profiles",
    "PushPolicy",
    "Request",
    "Response",
    "SYNAP",
    "ServerError",
    "ThunderError",
    "TimeoutError",
    "TlsPolicy",
    "VECTORIZER",
    "Value",
    "WIRE_VERSION",
    "from_server_message",
    "parse_endpoint",
    "registry",
    "wire",
]
