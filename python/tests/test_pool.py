"""Connection-pool behavior (CLT-080) — the Python mirror of
``rust/thunder/tests/pool.rs``, for both the sync :class:`Pool` and the async
:class:`AsyncPool` (FR-28).

The pool is a layer above the single-connection client; these tests exercise it
against the scripted loopback responders — checkout/return, the capacity bound,
the poison drop, and the property the whole layer exists for: ``N`` operations
pay **one** connection and **one** handshake, not ``N``."""

from __future__ import annotations

import asyncio
import threading

import pytest
from mockserver import SRV_CAP, MockServer

from thunder_rpc import (
    AsyncPool,
    Client,
    ClientConfig,
    Config,
    ErrorConvention,
    Handshake,
    HelloStyle,
    Pool,
    PushPolicy,
    TlsPolicy,
    Value,
)


def plain_config() -> Config:
    """No handshake — proves connection reuse via the accept count alone."""
    return Config(
        scheme="test",
        default_port=0,
        handshake=Handshake.NONE,
        hello_style=HelloStyle.NOT_USED,
        push=PushPolicy.RESERVED,
        max_frame_bytes=SRV_CAP,
        max_in_flight=64,
        error_codes=ErrorConvention.NONE,
        tls=TlsPolicy.OFF,
    )


def hello_config() -> Config:
    """The standard mandatory-HELLO shape, so a real handshake happens on
    every new connection — used to prove N ops pay one handshake."""
    return Config(
        scheme="test",
        default_port=0,
        handshake=Handshake.HELLO_MANDATORY,
        hello_style=HelloStyle.MAP_PAYLOAD,
        push=PushPolicy.RESERVED,
        max_frame_bytes=SRV_CAP,
        max_in_flight=64,
        error_codes=ErrorConvention.BRACKET_CODE,
        tls=TlsPolicy.OFF,
    )


def _hello_ok(conn) -> None:
    hello = conn.read_request()
    assert hello.command == "HELLO"
    conn.send_ok(hello.id, Value.map([(Value.str("authenticated"), Value.bool(True))]))


# -- sync --------------------------------------------------------------------


def test_checkout_returns_the_connection_for_reuse() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        req = conn.read_request()
        conn.send_ok(req.id, Value.str("PONG"))

    with MockServer(script) as srv:
        pool = Pool(srv.address, plain_config(), ClientConfig(), max_connections=4)
        assert pool.idle_count() == 0, "construction dials nothing"
        with pool.acquire() as client:
            assert client.call("PING").as_str() == "PONG"
            assert pool.idle_count() == 0, "checked out, so not idle"
        # The guard exited: the connection returned to the pool.
        assert pool.idle_count() == 1, "returned on exit"


def test_n_operations_use_one_connection_and_handshake() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        _hello_ok(conn)  # exactly one handshake for all N operations
        for _ in range(10):
            req = conn.read_request()
            conn.send_ok(req.id, Value.str("PONG"))

    with MockServer(script) as srv:
        pool = Pool(
            srv.address,
            hello_config(),
            ClientConfig(client_name="pool-test"),
            max_connections=4,
        )
        for _ in range(10):
            with pool.acquire() as client:
                assert client.call("PING").as_str() == "PONG"
        # Ten sequential operations reused one connection, so the server saw
        # one handshake, not ten.
        assert srv.accepts == 1, "ten operations must ride one connection"


def test_pool_never_exceeds_max_connections() -> None:
    def script(srv: MockServer) -> None:
        c1 = srv.accept()  # first checkout dials connection A
        srv.accept()  # second checkout dials connection B
        # The third checkout reuses connection A (idle after g1 released, LIFO);
        # one PING then arrives on it.
        req = c1.read_request()
        c1.send_ok(req.id, Value.str("PONG"))

    with MockServer(script) as srv:
        pool = Pool(srv.address, plain_config(), ClientConfig(), max_connections=2)

        g1 = pool.acquire()
        g1.__enter__()
        g2 = pool.acquire()
        g2.__enter__()

        # With both permits held, a third checkout must block, not open a
        # third connection (CLT-080 fixed N).
        third_client: list[Client] = []

        def take_third() -> None:
            g3 = pool.acquire()
            third_client.append(g3.__enter__())

        t = threading.Thread(target=take_third)
        t.start()
        t.join(timeout=0.15)
        assert t.is_alive(), "third checkout must block while max are held"

        # Release one; the waiter now completes, reusing connection 1.
        g1.__exit__(None, None, None)
        t.join(timeout=2.0)
        assert not t.is_alive(), "third checkout should proceed once a slot frees"
        assert third_client[0].call("PING").as_str() == "PONG"

        # At most two connections ever existed.
        assert srv.accepts <= 2


def test_a_poisoned_connection_is_not_handed_to_the_next_caller() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        req = conn.read_request()
        conn.send_ok(req.id, Value.str("PONG"))
        # The client killed connection 1; the next checkout dials fresh.
        conn2 = srv.accept()
        req2 = conn2.read_request()
        conn2.send_ok(req2.id, Value.str("PONG"))

    with MockServer(script) as srv:
        pool = Pool(srv.address, plain_config(), ClientConfig(), max_connections=4)

        with pool.acquire() as client:
            assert client.call("PING").as_str() == "PONG"
            client.close()
            assert not client.is_alive()
        # CLT-014: the dead connection was dropped, not parked for reuse.
        assert pool.idle_count() == 0, "a poisoned connection must not return"

        # The next checkout dials a fresh, working connection.
        with pool.acquire() as fresh:
            assert fresh.call("PING").as_str() == "PONG"


# -- async -------------------------------------------------------------------


async def test_checkout_returns_the_connection_for_reuse_async() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        req = conn.read_request()
        conn.send_ok(req.id, Value.str("PONG"))

    with MockServer(script) as srv:
        pool = AsyncPool(srv.address, plain_config(), ClientConfig(), max_connections=4)
        assert pool.idle_count() == 0, "construction dials nothing"
        async with pool.acquire() as client:
            assert (await client.call("PING")).as_str() == "PONG"
            assert pool.idle_count() == 0, "checked out, so not idle"
        assert pool.idle_count() == 1, "returned on exit"


async def test_n_operations_use_one_connection_and_handshake_async() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        _hello_ok(conn)
        for _ in range(10):
            req = conn.read_request()
            conn.send_ok(req.id, Value.str("PONG"))

    with MockServer(script) as srv:
        pool = AsyncPool(
            srv.address,
            hello_config(),
            ClientConfig(client_name="pool-test"),
            max_connections=4,
        )
        for _ in range(10):
            async with pool.acquire() as client:
                assert (await client.call("PING")).as_str() == "PONG"
        assert srv.accepts == 1, "ten operations must ride one connection"


async def test_pool_never_exceeds_max_connections_async() -> None:
    def script(srv: MockServer) -> None:
        c1 = srv.accept()  # first checkout dials connection A
        srv.accept()  # second checkout dials connection B
        # The third checkout reuses connection A (idle after g1 released, LIFO).
        req = c1.read_request()
        c1.send_ok(req.id, Value.str("PONG"))

    with MockServer(script) as srv:
        pool = AsyncPool(srv.address, plain_config(), ClientConfig(), max_connections=2)

        g1 = pool.acquire()
        await g1.__aenter__()
        g2 = pool.acquire()
        await g2.__aenter__()

        # With both permits held, a third checkout must block.
        g3 = pool.acquire()
        third = asyncio.ensure_future(g3.__aenter__())
        with pytest.raises(asyncio.TimeoutError):
            await asyncio.wait_for(asyncio.shield(third), timeout=0.15)

        # Release one; the waiter now completes, reusing connection 1.
        await g1.__aexit__(None, None, None)
        c3 = await asyncio.wait_for(third, timeout=2.0)
        assert (await c3.call("PING")).as_str() == "PONG"

        assert srv.accepts <= 2

        await g2.__aexit__(None, None, None)
        await g3.__aexit__(None, None, None)


async def test_a_poisoned_connection_is_not_handed_to_the_next_caller_async() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        req = conn.read_request()
        conn.send_ok(req.id, Value.str("PONG"))
        conn2 = srv.accept()
        req2 = conn2.read_request()
        conn2.send_ok(req2.id, Value.str("PONG"))

    with MockServer(script) as srv:
        pool = AsyncPool(srv.address, plain_config(), ClientConfig(), max_connections=4)

        async with pool.acquire() as client:
            assert (await client.call("PING")).as_str() == "PONG"
            await client.close()
            assert not client.is_alive()
        assert pool.idle_count() == 0, "a poisoned connection must not return"

        async with pool.acquire() as fresh:
            assert (await fresh.call("PING")).as_str() == "PONG"
