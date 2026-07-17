"""Behavioral floor tests for the asyncio client (SPEC-003, feeds CLT-090)
— the IDENTICAL contract as the sync suite (FR-28), same scripted loopback
responders, asyncio idiom only. Adds the asyncio-specific CLT-021 check:
cancellation removes the pending entry."""

from __future__ import annotations

import asyncio
import contextlib

import pytest
from mockserver import SRV_CAP, MockServer, PeerClosed

from thunder_rpc import (
    AsyncClient,
    ClientConfig,
    Config,
    Credentials,
    ErrorConvention,
    Handshake,
    HelloStyle,
    PushPolicy,
    TlsPolicy,
    Value,
    errors,
)


def plain_config(**overrides) -> Config:
    """A custom config (PRO-020): no handshake, push reserved, no error
    parsing — the neutral baseline the behavioral tests mutate. Thunder ships
    no application configs, so the tests build their own exactly as an
    application does."""
    fields = dict(
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
    fields.update(overrides)
    return Config(**fields)


def auth_command_config() -> Config:
    """A config with the ``AUTH_COMMAND`` shape and **no** HELLO — the shape a
    deployment whose RPC path authenticates via ``AUTH`` uses. Named for the
    shape: Thunder ships no application configs (PRO-020), so tests build
    their own exactly as an application does."""
    return plain_config(
        handshake=Handshake.AUTH_COMMAND,
        hello_style=HelloStyle.NOT_USED,
        error_codes=ErrorConvention.RESP3_PREFIXES,
    )


def argless_hello_config() -> Config:
    """The ``AUTH_COMMAND`` shape plus an optional arg-less HELLO."""
    return plain_config(
        handshake=Handshake.AUTH_COMMAND,
        hello_style=HelloStyle.ARG_LESS,
        error_codes=ErrorConvention.RESP3_PREFIXES,
    )


def hello_mandatory_config() -> Config:
    """The standard ``HELLO_MANDATORY`` + map-payload shape."""
    return plain_config(
        handshake=Handshake.HELLO_MANDATORY,
        hello_style=HelloStyle.MAP_PAYLOAD,
        error_codes=ErrorConvention.BRACKET_CODE,
    )


def hello_ok_reply() -> Value:
    return Value.map([(Value.str("authenticated"), Value.bool(True))])


# -- Multiplexing (CLT-010/011) ----------------------------------------------


async def test_pipelined_calls_complete_out_of_order() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        first = conn.read_request()
        second = conn.read_request()
        assert first.id != second.id, "ids must be distinct (CLT-010)"
        conn.send_ok(second.id, Value.str(second.command))
        conn.send_ok(first.id, Value.str(first.command))

    with MockServer(script) as srv:
        client = await AsyncClient.connect(srv.address, plain_config())
        one, two = await asyncio.gather(client.call("ONE"), client.call("TWO"))
        assert one.as_str() == "ONE"
        assert two.as_str() == "TWO"
        await client.close()


async def test_in_flight_bound_backpressures_instead_of_refusing() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        for _ in range(2):
            request = conn.read_request()
            conn.send_ok(request.id, Value.str(request.command))

    with MockServer(script) as srv:
        client = await AsyncClient.connect(srv.address, plain_config(max_in_flight=1))
        a, b = await asyncio.gather(client.call("A"), client.call("B"))
        assert a.as_str() == "A"
        assert b.as_str() == "B"
        await client.close()


async def test_stray_response_id_is_dropped_never_fatal() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        request = conn.read_request()
        conn.send_ok(9_999, Value.null())
        conn.send_ok(request.id, Value.str("real"))

    with MockServer(script) as srv:
        client = await AsyncClient.connect(srv.address, plain_config())
        value = await client.call("GET")
        assert value.as_str() == "real"
        assert client.unknown_response_drops() == 1
        await client.close()


# -- Handshakes (CLT-002/003) --------------------------------------------------


async def test_none_handshake_sends_nothing_before_user_calls() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        request = conn.read_request()
        assert request.command == "PING"
        conn.send_ok(request.id, Value.str("PONG"))

    with MockServer(script) as srv:
        # `plain_config()` is the genuine Handshake.NONE case. (This test
        # used to ride on auth_command_config(), which is AUTH_COMMAND since
        # BN-023.)
        client = await AsyncClient.connect(srv.address, plain_config())
        assert not client.is_authenticated()
        pong = await client.call("PING")
        assert pong.as_str() == "PONG"
        await client.close()


async def test_auth_command_without_hello_and_no_credentials_sends_nothing() -> None:
    """The client half of the shape/policy split BN-023 corrected: under
    ``AUTH_COMMAND``, a client with no credentials configured sends no
    ``AUTH`` at all — exactly right against an open deployment that does not
    require them. Enforcement is the server's policy, not the config's. It
    must also never send ``HELLO`` (``HelloStyle.NOT_USED``)."""

    def script(srv: MockServer) -> None:
        conn = srv.accept()
        request = conn.read_request()
        assert request.command == "PING", "no AUTH/HELLO frame without credentials"
        conn.send_ok(request.id, Value.str("PONG"))

    with MockServer(script) as srv:
        client = await AsyncClient.connect(srv.address, auth_command_config())
        assert not client.is_authenticated()
        pong = await client.call("PING")
        assert pong.as_str() == "PONG"
        await client.close()


async def test_auth_command_without_hello_sends_auth_and_never_hello() -> None:
    """BN-023 regression: the ``AUTH_COMMAND`` + ``HelloStyle.NOT_USED``
    shape must be able to authenticate.

    Deployments of this shape were once read as ``Handshake.NONE``, so a
    credentialed client sent **nothing** and could never reach a deployment
    that requires credentials. An RPC path with an ``AUTH`` handler and no
    ``HELLO`` handler is ``AUTH_COMMAND`` + ``HelloStyle.NOT_USED``: ``AUTH``
    goes out, ``HELLO`` never does."""

    def script(srv: MockServer) -> None:
        conn = srv.accept()
        # First frame must be AUTH — this shape has no HELLO handler at all.
        auth = conn.read_request()
        assert auth.command == "AUTH", "first frame must be AUTH, not HELLO"
        assert auth.args == (
            Value.str("root"),
            Value.str("hunter2"),
        ), "the AUTH <user> <password> form"
        conn.send_ok(auth.id, Value.str("OK"))
        ping = conn.read_request()
        assert ping.command == "PING"
        conn.send_ok(ping.id, Value.str("PONG"))

    with MockServer(script) as srv:
        config = ClientConfig(credentials=Credentials.user_pass("root", "hunter2"))
        client = await AsyncClient.connect(srv.address, auth_command_config(), config)
        assert client.is_authenticated()
        pong = await client.call("PING")
        assert pong.as_str() == "PONG"
        await client.close()


async def test_auth_command_handshake_sends_hello_then_auth_api_key() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        hello = conn.read_request()
        assert hello.command == "HELLO"
        assert hello.args == (), (
            "the arg-less RPC HELLO takes no arguments — the positional "
            "[Int(1)] is the RESP3 HELLO, a different surface (BN-023 errata)"
        )
        conn.send_ok(hello.id, Value.null())
        auth = conn.read_request()
        assert auth.command == "AUTH"
        assert auth.args == (Value.str("k-123"),)
        conn.send_ok(auth.id, Value.str("OK"))
        ping = conn.read_request()
        assert ping.command == "PING"
        conn.send_ok(ping.id, Value.str("PONG"))

    with MockServer(script) as srv:
        config = ClientConfig(credentials=Credentials.api_key("k-123"))
        client = await AsyncClient.connect(srv.address, argless_hello_config(), config)
        assert client.is_authenticated()
        pong = await client.call("PING")
        assert pong.as_str() == "PONG"
        await client.close()


async def test_auth_command_handshake_sends_user_pass() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        hello = conn.read_request()
        assert hello.command == "HELLO"
        conn.send_ok(hello.id, Value.null())
        auth = conn.read_request()
        assert auth.command == "AUTH"
        assert auth.args == (Value.str("admin"), Value.str("hunter2"))
        conn.send_ok(auth.id, Value.str("OK"))

    with MockServer(script) as srv:
        config = ClientConfig(credentials=Credentials.user_pass("admin", "hunter2"))
        client = await AsyncClient.connect(srv.address, argless_hello_config(), config)
        assert client.is_authenticated()
        await client.close()


async def test_auth_command_without_credentials_sends_nothing() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        request = conn.read_request()
        assert request.command == "PING", "no HELLO/AUTH without credentials"
        conn.send_ok(request.id, Value.str("PONG"))

    with MockServer(script) as srv:
        client = await AsyncClient.connect(srv.address, argless_hello_config())
        await client.call("PING")
        await client.close()


async def test_hello_mandatory_sends_hello_map_first_and_exposes_capabilities() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        hello = conn.read_request()
        assert hello.command == "HELLO", "HELLO must be the first frame"
        payload = hello.args[0]
        assert payload.map_get("version").as_int() == 1
        assert payload.map_get("token").as_str() == "tok-1"
        assert payload.map_get("client_name").as_str() == "itest"
        conn.send_ok(
            hello.id,
            Value.map(
                [
                    (Value.str("authenticated"), Value.bool(True)),
                    (
                        Value.str("capabilities"),
                        Value.array([Value.str("search"), Value.str("insert")]),
                    ),
                ]
            ),
        )

    with MockServer(script) as srv:
        config = ClientConfig(
            credentials=Credentials.token("tok-1"), client_name="itest"
        )
        client = await AsyncClient.connect(
            srv.address, hello_mandatory_config(), config
        )
        assert client.is_authenticated()
        assert client.capabilities() == ("search", "insert")
        await client.close()


async def test_handshake_rejection_is_a_typed_auth_error() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        hello = conn.read_request()
        conn.send_err(hello.id, "[unauthorized] invalid api key")

    with MockServer(script) as srv:
        config = ClientConfig(credentials=Credentials.api_key("wrong"))
        with pytest.raises(errors.AuthError) as excinfo:
            await AsyncClient.connect(srv.address, hello_mandatory_config(), config)
        assert "unauthorized" in str(excinfo.value)


# -- Timeouts and cancellation (CLT-020/021) --------------------------------------


async def test_per_call_timeout_fires_and_late_response_is_dropped() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        slow = conn.read_request()
        fresh = conn.read_request()
        conn.send_ok(slow.id, Value.str("late"))
        conn.send_ok(fresh.id, Value.str("fresh"))

    with MockServer(script) as srv:
        client = await AsyncClient.connect(srv.address, plain_config())
        with pytest.raises(errors.TimeoutError):
            await client.call("SLOW", timeout=0.1)
        value = await client.call("NEXT")
        assert value.as_str() == "fresh"
        assert client.unknown_response_drops() == 1
        await client.close()


async def test_cancellation_removes_the_pending_entry() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        slow = conn.read_request()
        # The next request proves the caller moved on after cancelling;
        # the late reply to the cancelled id must be dropped (CLT-013).
        fresh = conn.read_request()
        conn.send_ok(slow.id, Value.str("late"))
        conn.send_ok(fresh.id, Value.str("fresh"))

    with MockServer(script) as srv:
        client = await AsyncClient.connect(srv.address, plain_config())
        task = asyncio.create_task(client.call("SLOW"))
        await asyncio.sleep(0.15)  # let the request reach the wire
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
        # CLT-021: the pending entry is gone — the late response is an
        # unknown-id drop, and the connection lives on.
        value = await client.call("NEXT")
        assert value.as_str() == "fresh"
        assert client.unknown_response_drops() == 1
        await client.close()


# -- Reconnection (CLT-030/031) ----------------------------------------------------


async def test_reconnect_after_server_drop_succeeds() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        request = conn.read_request()
        conn.send_ok(request.id, Value.str("first"))
        conn.close()
        conn = srv.accept()
        request = conn.read_request()
        conn.send_ok(request.id, Value.str("second"))

    with MockServer(script) as srv:
        client = await AsyncClient.connect(srv.address, plain_config())
        first = await client.call("A")
        assert first.as_str() == "first"
        await asyncio.sleep(0.3)  # let the reader observe the EOF
        second = await client.call("B")
        assert second.as_str() == "second"
        await client.close()


async def test_reconnect_gives_up_after_two_attempts_with_typed_connection_error() -> (
    None
):
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        hello = conn.read_request()
        conn.send_ok(hello.id, hello_ok_reply())
        request = conn.read_request()
        conn.send_ok(request.id, Value.str("ok"))
        conn.close()
        for _ in range(2):
            srv.accept().close()

    with MockServer(script) as srv:
        config = ClientConfig(credentials=Credentials.api_key("k"))
        client = await AsyncClient.connect(
            srv.address, hello_mandatory_config(), config
        )
        await client.call("PING")
        await asyncio.sleep(0.3)

        with pytest.raises(errors.ConnectionError):
            await client.call("PING")
        assert (
            srv.accepts == 3
        ), "initial connect + exactly 2 re-dial attempts (CLT-030)"
        await client.close()


# -- Error mapping (CLT-050..052) ---------------------------------------------------


async def test_resp3_error_mapping_over_the_wire() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        request = conn.read_request()
        conn.send_err(request.id, "NOAUTH Authentication required.")
        request = conn.read_request()
        conn.send_err(request.id, "ERR unknown command 'FOO'")

    with MockServer(script) as srv:
        client = await AsyncClient.connect(srv.address, argless_hello_config())
        with pytest.raises(errors.AuthError) as auth_info:
            await client.call("GET")
        assert auth_info.value.message == "NOAUTH Authentication required."
        with pytest.raises(errors.ServerError) as server_info:
            await client.call("FOO")
        assert server_info.value.message == "ERR unknown command 'FOO'"
        assert server_info.value.code is None
        await client.close()


async def test_bracket_error_mapping_over_the_wire() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        hello = conn.read_request()
        conn.send_ok(hello.id, hello_ok_reply())
        request = conn.read_request()
        conn.send_err(request.id, "[collection_not_found] no such collection: docs")

    with MockServer(script) as srv:
        client = await AsyncClient.connect(srv.address, hello_mandatory_config())
        with pytest.raises(errors.ServerError) as excinfo:
            await client.call("SEARCH")
        assert (
            excinfo.value.message == "[collection_not_found] no such collection: docs"
        )
        assert excinfo.value.code == "collection_not_found"
        await client.close()


# -- Push frames (CLT-060) -----------------------------------------------------------


async def test_push_frames_route_to_handler_under_enabled() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        request = conn.read_request()
        conn.send_push(Value.str("evt"))
        conn.send_ok(request.id, Value.str("PONG"))

    with MockServer(script) as srv:
        client = await AsyncClient.connect(
            srv.address, plain_config(push=PushPolicy.ENABLED)
        )
        pushed: list[Value] = []
        client.on_push(pushed.append)
        pong = await client.call("SUBSCRIBE")
        assert pong.as_str() == "PONG"
        assert [v.as_str() for v in pushed] == ["evt"]
        assert client.unknown_response_drops() == 0
        await client.close()


async def test_push_frame_under_reserved_push_poisons_connection() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        conn.read_request()
        conn.send_push(Value.null())
        conn.close()
        conn = srv.accept()
        request = conn.read_request()
        conn.send_ok(request.id, Value.str("recovered"))

    with MockServer(script) as srv:
        client = await AsyncClient.connect(srv.address, plain_config())
        with pytest.raises(errors.DecodeError):
            await client.call("GET")
        value = await client.call("GET")
        assert value.as_str() == "recovered"
        await client.close()


# -- Poisoning (CLT-014) --------------------------------------------------------------


async def test_oversized_inbound_frame_fails_typed_and_poisons() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        conn.read_request()
        conn.send_raw((1_000).to_bytes(4, "little"))
        conn.close()
        conn = srv.accept()
        request = conn.read_request()
        conn.send_ok(request.id, Value.str("recovered"))

    with MockServer(script) as srv:
        client = await AsyncClient.connect(
            srv.address, plain_config(max_frame_bytes=64)
        )
        with pytest.raises(errors.FrameTooLargeError):
            await client.call("GET")
        value = await client.call("GET")
        assert value.as_str() == "recovered"
        await client.close()


async def test_malformed_frame_poisons_with_decode_error() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        conn.read_request()
        conn.send_raw((4).to_bytes(4, "little") + b"\xc1\xc1\xc1\xc1")

    with MockServer(script) as srv:
        client = await AsyncClient.connect(srv.address, plain_config())
        with pytest.raises(errors.DecodeError):
            await client.call("GET")
        await client.close()


# -- Lifecycle (CLT-004) --------------------------------------------------------------


async def test_close_is_idempotent_and_fails_in_flight_calls() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        conn.read_request()
        with contextlib.suppress(PeerClosed):
            conn.read_request()

    with MockServer(script) as srv:
        client = await AsyncClient.connect(srv.address, plain_config())
        pending = asyncio.create_task(client.call("HANG"))
        await asyncio.sleep(0.15)

        await client.close()
        await client.close()  # idempotent (CLT-004)

        with pytest.raises(errors.ConnectionError):
            await pending
        with pytest.raises(errors.ConnectionError):
            await client.call("AFTER")


# -- Endpoints (CLT-070) --------------------------------------------------------------


async def test_http_url_is_rejected_at_connect() -> None:
    with pytest.raises(errors.ConnectionError) as excinfo:
        await AsyncClient.connect("http://localhost:8080", plain_config())
    message = str(excinfo.value)
    assert "RPC-only" in message and "HTTP client" in message


async def test_async_context_manager_closes() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        request = conn.read_request()
        conn.send_ok(request.id, Value.str("PONG"))
        with contextlib.suppress(PeerClosed):
            conn.read_request()

    with MockServer(script) as srv:
        async with await AsyncClient.connect(srv.address, plain_config()) as client:
            pong = await client.call("PING")
            assert pong.as_str() == "PONG"
        with pytest.raises(errors.ConnectionError):
            await client.call("AFTER")
