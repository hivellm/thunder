"""Length-prefixed MessagePack frame codec (SPEC-001).

::

    +-------------------+--------------------------+
    |  length: u32 (LE) |  body: MessagePack bytes |
    +-------------------+--------------------------+
        4 bytes              length bytes

The body is a :class:`~thunder_rpc.value.Request` or
:class:`~thunder_rpc.value.Response` in the family's externally-tagged
encoding over the 8-variant :class:`~thunder_rpc.value.Value` model: unit
variants serialize as a bare string (``"Null"``), payload variants as a
single-key map (``{"Int": 42}``), and ``Response.result`` nests two one-key
maps (``{"Ok": {"Str": "PONG"}}`` — WIRE-003, corpus-pinned).

Canonicalization (SPEC-001 §2):

- ``Bytes`` is emitted as MessagePack **bin** (WIRE-010,
  ``use_bin_type=True``); the legacy int-array form decodes too (WIRE-011)
  and is never re-emitted.
- ``Request``/``Response`` are emitted as array-encoded structs (WIRE-012);
  map-shaped requests decode fine (WIRE-013); map-shaped responses are
  rejected (no family SDK ever emitted one).
- Integers pack in the shortest form; floats always pack as f64 preserving
  bit patterns (WIRE-014 — msgpack packs Python floats as float64).

The cap is validated against the length prefix **before** the body buffer is
touched (WIRE-020/021), so a hostile prefix cannot exhaust memory. Decoders
handle partial input by returning ``None`` ("need more bytes") and consume
exactly one frame per decode (WIRE-022).

This module is pure: no sockets, no timers, no config dependency (WIRE-030).
"""

from __future__ import annotations

from typing import Any, Union

import msgpack

from .errors import DecodeError, frame_too_large
from .value import DEFAULT_MAX_FRAME_BYTES, Request, Response, Value

_I64_MIN = -(2**63)
_I64_MAX = 2**63 - 1
_U32_MAX = 2**32 - 1

Message = Union[Request, Response]

# -- Value <-> externally-tagged MessagePack objects -------------------------


def _tag(value: Value) -> Any:
    """Value -> the externally-tagged object tree msgpack packs (WIRE-003)."""
    kind = value.kind
    if kind == "null":
        return "Null"
    if kind == "bool":
        return {"Bool": value.value}
    if kind == "int":
        return {"Int": value.value}
    if kind == "float":
        return {"Float": value.value}
    if kind == "bytes":
        return {"Bytes": value.value}
    if kind == "str":
        return {"Str": value.value}
    if kind == "array":
        return {"Array": [_tag(item) for item in value.value]}
    if kind == "map":
        return {"Map": [[_tag(k), _tag(v)] for k, v in value.value]}
    raise TypeError(f"unknown Value kind: {kind!r}")


def _untag(obj: Any) -> Value:
    """Externally-tagged object -> Value, normalizing legacy forms."""
    if isinstance(obj, str):
        if obj == "Null":
            return Value.null()
        raise DecodeError(f"bare string {obj!r} is not a Value variant")
    if isinstance(obj, dict) and len(obj) == 1:
        tag, payload = next(iter(obj.items()))
        if tag == "Bool":
            if type(payload) is bool:
                return Value("bool", payload)
            raise DecodeError("Bool variant needs a bool payload")
        if tag == "Int":
            if type(payload) is int and _I64_MIN <= payload <= _I64_MAX:
                return Value("int", payload)
            raise DecodeError(f"Int variant needs an i64 payload, got {payload!r}")
        if tag == "Float":
            if type(payload) is float:
                return Value("float", payload)
            if type(payload) is int:  # serde-style leniency: int widens to f64
                return Value("float", float(payload))
            raise DecodeError("Float variant needs a float payload")
        if tag == "Bytes":
            if isinstance(payload, (bytes, bytearray)):
                return Value("bytes", bytes(payload))
            if isinstance(payload, list):
                # WIRE-011: pre-Thunder legacy int-array Bytes, decode-only.
                return Value("bytes", _bytes_from_int_array(payload))
            raise DecodeError("Bytes variant needs bin or an int array")
        if tag == "Str":
            if isinstance(payload, str):
                return Value("str", payload)
            raise DecodeError("Str variant needs a str payload")
        if tag == "Array":
            if isinstance(payload, list):
                return Value("array", tuple(_untag(item) for item in payload))
            raise DecodeError("Array variant needs an array payload")
        if tag == "Map":
            if isinstance(payload, list):
                pairs = []
                for entry in payload:
                    if not isinstance(entry, list) or len(entry) != 2:
                        raise DecodeError("Map entry must be a [key, value] pair")
                    pairs.append((_untag(entry[0]), _untag(entry[1])))
                return Value("map", tuple(pairs))
            raise DecodeError("Map variant needs a pair-list payload")
        raise DecodeError(f"unknown Value variant tag {tag!r}")
    raise DecodeError(f"not a Value: {obj!r}")


def _bytes_from_int_array(items: list) -> bytes:
    for item in items:
        if type(item) is not int or not 0 <= item <= 255:
            raise DecodeError(
                f"legacy Bytes int-array holds a non-byte element: {item!r}"
            )
    return bytes(items)


# -- body codec ---------------------------------------------------------------


def encode_body(msg: Message) -> bytes:
    """Encode one Request/Response body (no length prefix)."""
    if isinstance(msg, Request):
        obj: Any = [msg.id, msg.command, [_tag(a) for a in msg.args]]
    elif isinstance(msg, Response):
        result = {"Ok": _tag(msg.ok)} if msg.err is None else {"Err": msg.err}
        obj = [msg.id, result]
    else:
        raise TypeError(
            f"encode_body needs a Request or Response, got {type(msg).__name__}"
        )
    return msgpack.packb(obj, use_bin_type=True)


def _unpack(body: bytes) -> Any:
    try:
        return msgpack.unpackb(body, raw=False)
    except Exception as exc:  # msgpack raises a family of unpack errors
        raise DecodeError(f"malformed MessagePack body: {exc}") from exc


def _check_frame_id(raw: Any) -> int:
    if type(raw) is not int or not 0 <= raw <= _U32_MAX:
        raise DecodeError(f"frame id must be a u32, got {raw!r}")
    return raw


def decode_request_body(body: bytes) -> Request:
    """Decode one Request body: array-encoded (WIRE-012) or the legacy
    map shape (WIRE-013, decode-only)."""
    obj = _unpack(body)
    if isinstance(obj, list):
        if len(obj) != 3:
            raise DecodeError(f"Request array needs 3 elements, got {len(obj)}")
        raw_id, command, args = obj
    elif isinstance(obj, dict):
        # WIRE-013: pre-Thunder legacy map-shaped Request (dynamic-language
        # donors emitted it).
        try:
            raw_id, command, args = obj["id"], obj["command"], obj["args"]
        except KeyError as exc:
            raise DecodeError(f"map-shaped Request is missing field {exc}") from exc
    else:
        raise DecodeError(
            f"Request body must be an array or map, got {type(obj).__name__}"
        )
    frame_id = _check_frame_id(raw_id)
    if not isinstance(command, str):
        raise DecodeError(f"Request command must be a str, got {command!r}")
    if not isinstance(args, list):
        raise DecodeError("Request args must be an array")
    return Request(id=frame_id, command=command, args=tuple(_untag(a) for a in args))


def decode_response_body(body: bytes) -> Response:
    """Decode one Response body. ``result`` is the externally-tagged
    ``{"Ok": <value>}`` / ``{"Err": "<string>"}`` (WIRE-003). Map-shaped
    responses are rejected (WIRE-013 allows it — no family SDK emits them).
    """
    obj = _unpack(body)
    if not isinstance(obj, list) or len(obj) != 2:
        raise DecodeError("Response body must be a 2-element array [id, result]")
    frame_id = _check_frame_id(obj[0])
    result = obj[1]
    if isinstance(result, dict) and len(result) == 1:
        arm, payload = next(iter(result.items()))
        if arm == "Ok":
            return Response(id=frame_id, ok=_untag(payload))
        if arm == "Err":
            if not isinstance(payload, str):
                raise DecodeError("Err arm must carry a string")
            return Response(id=frame_id, err=payload)
    raise DecodeError(
        f"Response result must be {{'Ok': ...}} or {{'Err': ...}}, got {result!r}"
    )


# -- frame codec --------------------------------------------------------------


def encode_frame(msg: Message, *, max_frame_bytes: int | None = None) -> bytes:
    """Encode a message into one complete frame (``u32 LE length`` + body).

    When ``max_frame_bytes`` is given, a body over the cap raises the typed
    :class:`~thunder_rpc.errors.FrameTooLargeError` before anything is sent
    (WIRE-020 applies on encode too).
    """
    body = encode_body(msg)
    limit = max_frame_bytes if max_frame_bytes is not None else _U32_MAX
    if len(body) > limit:
        raise frame_too_large(len(body), limit)
    return len(body).to_bytes(4, "little") + body


def try_split_frame(
    buf: bytes, *, max_frame_bytes: int = DEFAULT_MAX_FRAME_BYTES
) -> tuple[bytes, int] | None:
    """Split one frame off ``buf``: ``(body, bytes_consumed)``.

    Returns ``None`` when the buffer does not yet hold a complete frame
    (read more and retry — WIRE-022). The cap is validated against the
    length prefix **before** the body is touched (WIRE-020/021).
    """
    if len(buf) < 4:
        return None
    length = int.from_bytes(buf[:4], "little")
    if length > max_frame_bytes:
        raise frame_too_large(length, max_frame_bytes)
    total = 4 + length
    if len(buf) < total:
        return None
    return bytes(buf[4:total]), total


def decode_request(
    buf: bytes, *, max_frame_bytes: int = DEFAULT_MAX_FRAME_BYTES
) -> tuple[Request, int] | None:
    """Decode one Request frame; ``None`` means "need more bytes"."""
    split = try_split_frame(buf, max_frame_bytes=max_frame_bytes)
    if split is None:
        return None
    body, consumed = split
    return decode_request_body(body), consumed


def decode_response(
    buf: bytes, *, max_frame_bytes: int = DEFAULT_MAX_FRAME_BYTES
) -> tuple[Response, int] | None:
    """Decode one Response frame; ``None`` means "need more bytes"."""
    split = try_split_frame(buf, max_frame_bytes=max_frame_bytes)
    if split is None:
        return None
    body, consumed = split
    return decode_response_body(body), consumed
