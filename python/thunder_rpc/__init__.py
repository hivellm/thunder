"""hivellm-thunder — HiveLLM binary RPC (wire v1, frozen).

One frame is ``u32 LE length`` + MessagePack body; the body is a
:class:`Request` or :class:`Response` in the family's externally-tagged
encoding over the 8-variant :class:`Value` model (SPEC-001). A
:class:`Config` (SPEC-002) describes how **one application** uses the shared
wire — Thunder ships one standard and no product knowledge — and the
:class:`Client` (threads) and :class:`AsyncClient` (asyncio) implement the
uniform client contract (SPEC-003) on top.

Quickstart::

    from thunder_rpc import Client, Config, Value

    # The application's own identity on top of the family standard.
    config = Config.standard().with_scheme("myapp").with_port(9000)

    with Client.connect("myapp://localhost", config) as client:
        pong = client.call("PING")
        assert pong.as_str() == "PONG"
"""

from . import wire
from .aio import AsyncClient
from .client import Client
from .client_config import ClientConfig, Credentials, HandshakeInfo
from .config import (
    Config,
    ErrorConvention,
    Handshake,
    HelloStyle,
    PushPolicy,
    TlsPolicy,
)
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
from .pool import AsyncPool, Pool
from .tls import ClientTls
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
    "AsyncPool",
    "AuthError",
    "Client",
    "ClientConfig",
    "ClientTls",
    "Config",
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
    "PUSH_ID",
    "Pool",
    "PushPolicy",
    "Request",
    "Response",
    "ServerError",
    "ThunderError",
    "TimeoutError",
    "TlsPolicy",
    "Value",
    "WIRE_VERSION",
    "from_server_message",
    "parse_endpoint",
    "wire",
]
