"""The 8-variant value model and the ``Request``/``Response`` frame bodies.

Mirrors ``thunder-wire``'s ``value.rs`` (WIRE-001/002): externally-tagged
encoding is handled by :mod:`thunder_rpc.wire`; this module is the pure data
model. ``Map`` is an insertion-ordered pair list because keys may be any
value (WIRE-002).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable

#: Negotiated wire protocol version. v1 is the only version anywhere (WIRE-004).
WIRE_VERSION = 1

#: Reserved frame id for server-initiated push frames (WIRE-005).
PUSH_ID = 0xFFFF_FFFF

#: Default frame-body cap: 64 MiB, checked before allocation (WIRE-020).
DEFAULT_MAX_FRAME_BYTES = 64 * 1024 * 1024

_I64_MIN = -(2**63)
_I64_MAX = 2**63 - 1
_U32_MAX = 2**32 - 1

#: The 8 wire value kinds (WIRE-002).
KINDS = ("null", "bool", "int", "float", "bytes", "str", "array", "map")


@dataclass(frozen=True)
class Value:
    """One wire value — byte-compatible with ``SynapValue`` / ``NexusValue`` /
    ``VectorizerValue`` (WIRE-002).

    Construct through the factories (``Value.int(42)``, ``Value.map(pairs)``,
    ...) which validate and normalize; ``kind`` is one of :data:`KINDS`.
    """

    kind: str
    value: Any

    # -- factories ----------------------------------------------------------

    @staticmethod
    def null() -> Value:
        """SQL NULL / nil."""
        return Value("null", None)

    @staticmethod
    def bool(value: bool) -> Value:  # noqa: A003 - mirrors the wire tag names
        if type(value) is not bool:
            raise TypeError(f"Value.bool needs a bool, got {type(value).__name__}")
        return Value("bool", value)

    @staticmethod
    def int(value: int) -> Value:
        if type(value) is not int:
            raise TypeError(f"Value.int needs an int, got {type(value).__name__}")
        if not _I64_MIN <= value <= _I64_MAX:
            raise ValueError(f"Value.int must fit i64, got {value}")
        return Value("int", value)

    @staticmethod
    def float(value: float) -> Value:
        if type(value) is bool or not isinstance(value, (int, float)):
            raise TypeError(f"Value.float needs a float, got {type(value).__name__}")
        return Value("float", float(value))

    @staticmethod
    def bytes(data: bytes) -> Value:
        if not isinstance(data, (bytes, bytearray, memoryview)):
            raise TypeError(f"Value.bytes needs bytes, got {type(data).__name__}")
        return Value("bytes", bytes(data))

    @staticmethod
    def str(text: str) -> Value:
        if not isinstance(text, str):
            raise TypeError(f"Value.str needs a str, got {type(text).__name__}")
        return Value("str", text)

    @staticmethod
    def array(items: Iterable[Value]) -> Value:
        items = tuple(items)
        for item in items:
            if not isinstance(item, Value):
                raise TypeError(
                    f"Value.array items must be Value, got {type(item).__name__}"
                )
        return Value("array", items)

    @staticmethod
    def map(pairs: Iterable[tuple[Value, Value]]) -> Value:
        normalized = []
        for pair in pairs:
            key, val = pair
            if not isinstance(key, Value) or not isinstance(val, Value):
                raise TypeError("Value.map pairs must be (Value, Value) tuples")
            normalized.append((key, val))
        return Value("map", tuple(normalized))

    # -- accessors (mirror thunder-wire's Value accessors) -------------------

    def as_str(self) -> str | None:
        """Extract the inner string."""
        return self.value if self.kind == "str" else None

    def as_bytes(self) -> bytes | None:
        """Extract bytes (also accepts ``Str`` as UTF-8 bytes)."""
        if self.kind == "bytes":
            return self.value
        if self.kind == "str":
            return self.value.encode("utf-8")
        return None

    def as_int(self) -> int | None:
        """Extract an integer."""
        return self.value if self.kind == "int" else None

    def as_float(self) -> float | None:
        """Extract a float (accepts ``Int`` widened to float)."""
        if self.kind == "float":
            return self.value
        if self.kind == "int":
            return float(self.value)
        return None

    def as_bool(self) -> bool | None:
        """Extract a bool."""
        return self.value if self.kind == "bool" else None

    def as_array(self) -> tuple[Value, ...] | None:
        """Extract the array items."""
        return self.value if self.kind == "array" else None

    def as_map(self) -> tuple[tuple[Value, Value], ...] | None:
        """Extract the map pairs."""
        return self.value if self.kind == "map" else None

    def map_get(self, key: str) -> Value | None:
        """Look up a string key in a ``Map`` value."""
        pairs = self.as_map()
        if pairs is None:
            return None
        for k, v in pairs:
            if k.as_str() == key:
                return v
        return None

    def is_null(self) -> bool:
        """True for ``Value.null()``."""
        return self.kind == "null"


def _check_id(frame_id: int) -> None:
    if type(frame_id) is not int or not 0 <= frame_id <= _U32_MAX:
        raise ValueError(f"frame id must be a u32, got {frame_id!r}")


@dataclass(frozen=True)
class Request:
    """One RPC request (WIRE-001). ``id`` is client-chosen and echoed back;
    many requests multiplex over one connection. Serialized as an array
    (WIRE-012); map-shaped requests decode too (WIRE-013).
    """

    id: int
    command: str
    args: tuple[Value, ...] = ()

    def __post_init__(self) -> None:
        _check_id(self.id)
        if not isinstance(self.command, str):
            raise TypeError(f"command must be a str, got {type(self.command).__name__}")
        args = tuple(self.args)
        for arg in args:
            if not isinstance(arg, Value):
                raise TypeError(f"args items must be Value, got {type(arg).__name__}")
        object.__setattr__(self, "args", args)


@dataclass(frozen=True)
class Response:
    """One RPC response (WIRE-001). Exactly one of ``ok`` / ``err`` is set;
    v1 carries no structured error object — conventions are prefix-based and
    profile-driven (WIRE-040).
    """

    id: int
    ok: Value | None = None
    err: str | None = None

    def __post_init__(self) -> None:
        _check_id(self.id)
        if (self.ok is None) == (self.err is None):
            raise ValueError("Response needs exactly one of ok/err")
        if self.ok is not None and not isinstance(self.ok, Value):
            raise TypeError(f"ok must be a Value, got {type(self.ok).__name__}")
        if self.err is not None and not isinstance(self.err, str):
            raise TypeError(f"err must be a str, got {type(self.err).__name__}")
