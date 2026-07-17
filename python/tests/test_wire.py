"""Wire-layer unit tests beyond the corpus (mirrors thunder-wire's inline
suite): golden vectors, constants, factory/accessor validation, framing
edges, and the decode tolerances."""

from __future__ import annotations

import msgpack
import pytest

from thunder_rpc import (
    DEFAULT_MAX_FRAME_BYTES,
    PUSH_ID,
    WIRE_VERSION,
    Request,
    Response,
    Value,
    wire,
)
from thunder_rpc.errors import DecodeError, FrameTooLargeError


def hex_str(data: bytes) -> str:
    return " ".join(f"{b:02x}" for b in data)


def test_wire_constants() -> None:
    assert WIRE_VERSION == 1
    assert PUSH_ID == 0xFFFF_FFFF
    assert DEFAULT_MAX_FRAME_BYTES == 64 * 1024 * 1024


def test_ping_request_matches_family_golden_vector() -> None:
    request = Request(id=1, command="PING")
    frame = wire.encode_frame(request)
    assert hex_str(frame) == "08 00 00 00 93 01 a4 50 49 4e 47 90"
    decoded, consumed = wire.decode_request(frame)
    assert decoded == request
    assert consumed == len(frame)


def test_pong_response_matches_nested_ok_golden_vector() -> None:
    response = Response(id=1, ok=Value.str("PONG"))
    frame = wire.encode_frame(response)
    # Result nests two one-key maps: {"Ok": {"Str": "PONG"}} (WIRE-003).
    assert (
        hex_str(frame) == "10 00 00 00 92 01 81 a2 4f 6b 81 a3 53 74 72 a4 50 4f 4e 47"
    )
    decoded, _ = wire.decode_response(frame)
    assert decoded == response


def test_value_factories_validate() -> None:
    with pytest.raises(TypeError):
        Value.bool(1)
    with pytest.raises(TypeError):
        Value.int(True)
    with pytest.raises(ValueError):
        Value.int(2**63)
    with pytest.raises(TypeError):
        Value.float("1.5")
    with pytest.raises(TypeError):
        Value.str(b"x")
    with pytest.raises(TypeError):
        Value.bytes("x")
    with pytest.raises(TypeError):
        Value.array([1])
    with pytest.raises(TypeError):
        Value.map([(Value.str("k"), "v")])
    # Widening conveniences.
    assert Value.float(1).value == 1.0
    assert Value.bytes(bytearray(b"\x01")).value == b"\x01"


def test_value_accessors_mirror_reference() -> None:
    assert Value.str("x").as_str() == "x"
    assert Value.int(1).as_str() is None
    assert Value.bytes(b"\x00").as_bytes() == b"\x00"
    assert Value.str("hi").as_bytes() == b"hi"  # Str doubles as UTF-8 bytes
    assert Value.int(7).as_int() == 7
    assert Value.float(1.5).as_int() is None
    assert Value.float(1.5).as_float() == 1.5
    assert Value.int(2).as_float() == 2.0  # Int widens to float
    assert Value.bool(True).as_bool() is True
    assert Value.array([Value.int(1)]).as_array() == (Value.int(1),)
    pairs = ((Value.str("k"), Value.int(1)),)
    assert Value.map(pairs).as_map() == pairs
    assert Value.map(pairs).map_get("k") == Value.int(1)
    assert Value.map(pairs).map_get("missing") is None
    assert Value.null().is_null()
    assert not Value.int(0).is_null()


def test_request_and_response_validate() -> None:
    with pytest.raises(ValueError):
        Request(id=-1, command="PING")
    with pytest.raises(ValueError):
        Request(id=2**32, command="PING")
    with pytest.raises(TypeError):
        Request(id=1, command="PING", args=("nope",))
    with pytest.raises(ValueError):
        Response(id=1)  # neither ok nor err
    with pytest.raises(ValueError):
        Response(id=1, ok=Value.null(), err="both")


def test_encode_cap_applies_when_given() -> None:
    """WIRE-020 holds on encode too: a body over the cap never leaves."""
    response = Response(id=1, ok=Value.bytes(b"\x00" * 64))
    with pytest.raises(FrameTooLargeError):
        wire.encode_frame(response, max_frame_bytes=8)
    assert wire.encode_frame(response, max_frame_bytes=1024)


def test_oversized_prefix_rejected_before_body_arrives() -> None:
    """Only the 4-byte prefix claiming cap+1: the check fires without the
    body being present at all — allocation cannot have happened."""
    buf = (DEFAULT_MAX_FRAME_BYTES + 1).to_bytes(4, "little")
    with pytest.raises(FrameTooLargeError) as excinfo:
        wire.decode_request(buf)
    assert excinfo.value.body == DEFAULT_MAX_FRAME_BYTES + 1
    assert excinfo.value.limit == DEFAULT_MAX_FRAME_BYTES
    assert "exceeds limit" in str(excinfo.value)


def test_partial_input_returns_none_at_every_cut() -> None:
    frame = wire.encode_frame(Request(id=1, command="PING"))
    for cut in (0, 1, 3, 4, len(frame) - 1):
        assert wire.decode_request(frame[:cut]) is None, f"cut at {cut}"


def test_two_frames_in_one_buffer_consume_exactly_one_each() -> None:
    a = wire.encode_frame(Response(id=1, ok=Value.int(1)))
    b = wire.encode_frame(Response(id=2, ok=Value.int(2)))
    buf = a + b
    first, used = wire.decode_response(buf)
    assert first.id == 1
    assert used == len(a)
    second, used2 = wire.decode_response(buf[used:])
    assert second.id == 2
    assert used2 == len(b)


def test_garbage_body_is_a_typed_error_not_a_crash() -> None:
    buf = (4).to_bytes(4, "little") + b"\xc1\xc1\xc1\xc1"  # 0xc1 is never valid
    with pytest.raises(DecodeError):
        wire.decode_request(buf)


def test_map_shaped_response_is_rejected() -> None:
    """WIRE-013: map-shaped Requests decode; map-shaped Responses MAY be
    rejected — no family SDK ever emitted one, so Thunder rejects them."""
    body = msgpack.packb({"id": 1, "result": {"Ok": "Null"}}, use_bin_type=True)
    frame = len(body).to_bytes(4, "little") + body
    with pytest.raises(DecodeError):
        wire.decode_response(frame)


def test_legacy_bytes_int_array_rejects_out_of_range_elements() -> None:
    body = msgpack.packb([1, {"Ok": {"Bytes": [1, 256]}}], use_bin_type=True)
    frame = len(body).to_bytes(4, "little") + body
    with pytest.raises(DecodeError):
        wire.decode_response(frame)


def test_nan_bit_pattern_survives_round_trip() -> None:
    import struct

    nan = struct.unpack(">d", bytes.fromhex("7ff8dead00000000"))[0]
    frame = wire.encode_frame(Response(id=1, ok=Value.float(nan)))
    decoded, _ = wire.decode_response(frame)
    assert struct.pack(">d", decoded.ok.value) == struct.pack(">d", nan)
