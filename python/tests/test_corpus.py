"""Conformance-corpus loader (TST-020): walks ``conformance/vectors/`` and
asserts every vector per its ``mode``. Runs in the default test command —
never feature-gated, never skipped (NFR-03).

Mode semantics (TST-002, conformance/README.md):

- ``bidirectional`` — ``encode(decoded) == frame`` byte-exact AND
  ``decode(frame) == decoded`` structurally (floats by bit pattern).
- ``decode-only``   — decode succeeds and equals ``decoded``; the canonical
  encoding of ``decoded`` must NOT reproduce these legacy bytes.
- ``stream``        — ``frames`` decode back-to-back, one frame per decode,
  consuming the buffer exactly.
- ``incomplete``    — decoder asks for more bytes (no value, no error).
- ``reject``        — decode fails with the named ``error`` class.

``max_frame_bytes`` (optional) overrides the 64 MiB default cap so the
at-cap / over-cap boundary is testable with real bytes.
"""

from __future__ import annotations

import struct
from pathlib import Path

import pytest
import yaml

from thunder_rpc import DEFAULT_MAX_FRAME_BYTES, Request, Response, Value, wire
from thunder_rpc.errors import DecodeError, FrameTooLargeError

VECTOR_DIR = Path(__file__).resolve().parents[2] / "conformance" / "vectors"
VECTOR_PATHS = sorted(VECTOR_DIR.glob("*.yaml"))


def parse_hex(text: str) -> bytes:
    return bytes(int(byte, 16) for byte in text.split())


def node_to_value(node: dict) -> Value:
    """One ``decoded`` value node: ``{type, value}`` plus an optional
    ``bits`` field for floats — the u64 IEEE-754 bit pattern in hex,
    required for NaN and -0.0 where numeric equality cannot pin the wire
    bytes."""
    kind = node["type"]
    if kind == "null":
        return Value.null()
    if kind == "bool":
        return Value.bool(node["value"])
    if kind == "int":
        return Value.int(node["value"])
    if kind == "float":
        bits = node.get("bits")
        if bits is not None:
            return Value.float(struct.unpack(">d", bytes.fromhex(bits))[0])
        return Value.float(node["value"])
    if kind == "str":
        return Value.str(node["value"])
    if kind == "bytes":
        return Value.bytes(parse_hex(node["value"]))
    if kind == "array":
        return Value.array(node_to_value(item) for item in node["value"])
    if kind == "map":
        return Value.map((node_to_value(k), node_to_value(v)) for k, v in node["value"])
    raise AssertionError(f"unknown corpus node type: {kind}")


def expected_message(decoded: dict):
    if decoded["kind"] == "request":
        return Request(
            id=decoded["id"],
            command=decoded["command"],
            args=tuple(node_to_value(node) for node in decoded["args"]),
        )
    if decoded["kind"] == "response":
        if "ok" in decoded:
            return Response(id=decoded["id"], ok=node_to_value(decoded["ok"]))
        return Response(id=decoded["id"], err=decoded["err"])
    raise AssertionError(f"unknown decoded kind: {decoded['kind']}")


def values_eq(a: Value, b: Value) -> bool:
    """Structural equality with floats compared by u64 bit pattern — NaN
    never compares equal numerically and ``-0.0 == 0.0`` would hide the
    sign bit."""
    if a.kind != b.kind:
        return False
    if a.kind == "float":
        return struct.pack("<d", a.value) == struct.pack("<d", b.value)
    if a.kind == "array":
        return len(a.value) == len(b.value) and all(
            values_eq(x, y) for x, y in zip(a.value, b.value)
        )
    if a.kind == "map":
        return len(a.value) == len(b.value) and all(
            values_eq(ka, kb) and values_eq(va, vb)
            for (ka, va), (kb, vb) in zip(a.value, b.value)
        )
    return a.value == b.value


def messages_eq(got, want) -> bool:
    if isinstance(want, Request):
        return (
            isinstance(got, Request)
            and got.id == want.id
            and got.command == want.command
            and len(got.args) == len(want.args)
            and all(values_eq(g, w) for g, w in zip(got.args, want.args))
        )
    if not isinstance(got, Response) or got.id != want.id:
        return False
    if want.err is not None:
        return got.err == want.err
    return got.err is None and values_eq(got.ok, want.ok)


def decode_one(buf: bytes, kind: str, max_frame_bytes: int):
    if kind == "request":
        return wire.decode_request(buf, max_frame_bytes=max_frame_bytes)
    return wire.decode_response(buf, max_frame_bytes=max_frame_bytes)


@pytest.mark.parametrize("path", VECTOR_PATHS, ids=[p.stem for p in VECTOR_PATHS])
def test_corpus_vector(path: Path) -> None:
    vector = yaml.safe_load(path.read_text(encoding="utf-8"))
    name = vector["name"]
    frame = parse_hex(vector["frame_hex"])
    cap = vector.get("max_frame_bytes", DEFAULT_MAX_FRAME_BYTES)
    mode = vector["mode"]

    if mode == "bidirectional":
        want = expected_message(vector["decoded"])
        # encode(decoded) == frame, byte-exact.
        assert wire.encode_frame(want) == frame, f"{name}: encode mismatch"
        # decode(frame) == decoded, structurally (floats by bits).
        out = decode_one(frame, vector["decoded"]["kind"], cap)
        assert out is not None, f"{name}: frame must decode"
        got, consumed = out
        assert messages_eq(got, want), f"{name}: decode mismatch: {got!r} != {want!r}"
        assert consumed == len(frame), f"{name}: consumed"
    elif mode == "decode-only":
        want = expected_message(vector["decoded"])
        out = decode_one(frame, vector["decoded"]["kind"], cap)
        assert out is not None, f"{name}: legacy frame must decode"
        got, consumed = out
        assert messages_eq(got, want), f"{name}: decode mismatch: {got!r} != {want!r}"
        assert consumed == len(frame), f"{name}: consumed"
        # Encoding this form is forbidden: the canonical encoding of the
        # same structure must NOT reproduce the legacy bytes (WIRE-011/013).
        assert (
            wire.encode_frame(want) != frame
        ), f"{name}: legacy form must not be re-emitted"
    elif mode == "stream":
        offset = 0
        for index, decoded in enumerate(vector["frames"]):
            out = decode_one(frame[offset:], decoded["kind"], cap)
            assert out is not None, f"{name}[{index}]: must decode"
            got, consumed = out
            assert messages_eq(
                got, expected_message(decoded)
            ), f"{name}[{index}]: mismatch"
            offset += consumed
        assert offset == len(frame), f"{name}: buffer fully consumed"
    elif mode == "incomplete":
        out = wire.decode_request(frame, max_frame_bytes=cap)
        assert out is None, f"{name}: must ask for more bytes"
    elif mode == "reject":
        error_class = {"frame_too_large": FrameTooLargeError, "decode": DecodeError}[
            vector["error"]
        ]
        with pytest.raises(error_class):
            wire.decode_request(frame, max_frame_bytes=cap)
    else:
        raise AssertionError(f"{name}: unknown mode {mode}")


def test_corpus_floor() -> None:
    """The corpus must not silently shrink (TST-020; 1.0 floor is 38)."""
    assert len(VECTOR_PATHS) >= 38, f"found {len(VECTOR_PATHS)} vectors, floor is 38"
