"""hivellm-thunder — HiveLLM binary RPC (wire v1, frozen).

Skeleton package: the wire codec plus sync and async clients land at
DAG T3.2 (``phase3_python-package``), corpus-first per SPEC-005.
"""

__version__ = "0.1.0"

#: Negotiated wire protocol version. v1 is the only version anywhere.
WIRE_VERSION = 1

#: Reserved frame id for server push frames (WIRE-005).
PUSH_ID = 0xFFFF_FFFF

#: Default frame-body cap: 64 MiB, checked before allocation (WIRE-020).
DEFAULT_MAX_FRAME_BYTES = 64 * 1024 * 1024
