"""Skeleton sanity — the real suite lands at DAG T3.2 (corpus-first)."""

import thunder_rpc


def test_wire_constants():
    assert thunder_rpc.WIRE_VERSION == 1
    assert thunder_rpc.PUSH_ID == 0xFFFF_FFFF
    assert thunder_rpc.DEFAULT_MAX_FRAME_BYTES == 64 * 1024 * 1024
