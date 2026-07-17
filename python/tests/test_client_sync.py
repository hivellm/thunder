"""Behavioral floor tests for the sync client (SPEC-003, feeds CLT-090) —
the scenario-for-scenario mirror of ``rust/thunder-client/tests/behavior.rs``
against the scripted loopback responders in :mod:`mockserver`."""

from __future__ import annotations

import contextlib
import threading
import time
from concurrent.futures import ThreadPoolExecutor

import pytest
from mockserver import SRV_CAP, MockServer, PeerClosed

from thunder_rpc import (
    NEXUS,
    SYNAP,
    VECTORIZER,
    Client,
    ClientConfig,
    Credentials,
    Profile,
    Value,
    errors,
)
from thunder_rpc.profile import (
    ErrorConvention,
    Handshake,
    HelloStyle,
    PushPolicy,
    TlsPolicy,
)


def plain_profile(**overrides) -> Profile:
    """A custom profile (PRO-020): no handshake, push reserved, no error
    parsing — the neutral baseline the behavioral tests mutate."""
    fields = dict(
        name="test",
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
    return Profile(**fields)


def hello_ok_reply() -> Value:
    return Value.map([(Value.str("authenticated"), Value.bool(True))])


# -- Multiplexing (CLT-010/011) ----------------------------------------------


def test_pipelined_calls_complete_out_of_order() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        # Read BOTH requests before answering, then answer in reverse:
        # completion order follows the server, not submission order.
        first = conn.read_request()
        second = conn.read_request()
        assert first.id != second.id, "ids must be distinct (CLT-010)"
        conn.send_ok(second.id, Value.str(second.command))
        conn.send_ok(first.id, Value.str(first.command))

    with MockServer(script) as srv:
        client = Client.connect(srv.address, plain_profile())
        with ThreadPoolExecutor(max_workers=2) as pool:
            one = pool.submit(client.call, "ONE")
            two = pool.submit(client.call, "TWO")
            assert one.result(timeout=5).as_str() == "ONE"
            assert two.result(timeout=5).as_str() == "TWO"
        client.close()


def test_in_flight_bound_backpressures_instead_of_refusing() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        # Strictly serial: with max_in_flight = 1 the second call must wait
        # for the first permit, never be refused (CLT-012).
        for _ in range(2):
            request = conn.read_request()
            conn.send_ok(request.id, Value.str(request.command))

    with MockServer(script) as srv:
        client = Client.connect(srv.address, plain_profile(max_in_flight=1))
        with ThreadPoolExecutor(max_workers=2) as pool:
            a = pool.submit(client.call, "A")
            b = pool.submit(client.call, "B")
            assert a.result(timeout=5).as_str() == "A"
            assert b.result(timeout=5).as_str() == "B"
        client.close()


def test_stray_response_id_is_dropped_never_fatal() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        request = conn.read_request()
        # A response nobody asked for, then the real one (CLT-013).
        conn.send_ok(9_999, Value.null())
        conn.send_ok(request.id, Value.str("real"))

    with MockServer(script) as srv:
        client = Client.connect(srv.address, plain_profile())
        assert client.call("GET").as_str() == "real"
        assert client.unknown_response_drops() == 1
        client.close()


# -- Handshakes (CLT-002/003) --------------------------------------------------


def test_none_handshake_sends_nothing_before_user_calls() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        # The very first frame must be the user's command — no HELLO, no
        # AUTH (Handshake.NONE).
        request = conn.read_request()
        assert request.command == "PING"
        conn.send_ok(request.id, Value.str("PONG"))

    with MockServer(script) as srv:
        client = Client.connect(srv.address, SYNAP)
        assert not client.is_authenticated()
        assert client.call("PING").as_str() == "PONG"
        client.close()


def test_auth_command_handshake_sends_hello_then_auth_api_key() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        hello = conn.read_request()
        assert hello.command == "HELLO"
        assert hello.args == (Value.int(1),), "positional [Int(1)]"
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
        client = Client.connect(srv.address, NEXUS, config)
        assert client.is_authenticated()
        assert client.call("PING").as_str() == "PONG"
        client.close()


def test_auth_command_handshake_sends_user_pass() -> None:
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
        client = Client.connect(srv.address, NEXUS, config)
        assert client.is_authenticated()
        client.close()


def test_auth_command_without_credentials_sends_nothing() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        request = conn.read_request()
        assert request.command == "PING", "no HELLO/AUTH without credentials"
        conn.send_ok(request.id, Value.str("PONG"))

    with MockServer(script) as srv:
        client = Client.connect(srv.address, NEXUS)
        client.call("PING")
        client.close()


def test_hello_mandatory_sends_hello_map_first_and_exposes_capabilities() -> None:
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
        client = Client.connect(srv.address, VECTORIZER, config)
        assert client.is_authenticated()
        assert client.capabilities() == ("search", "insert")
        client.close()


def test_handshake_rejection_is_a_typed_auth_error() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        hello = conn.read_request()
        conn.send_err(hello.id, "[unauthorized] invalid api key")

    with MockServer(script) as srv:
        config = ClientConfig(credentials=Credentials.api_key("wrong"))
        # CLT-003: an auth failure is the auth class, not a generic error.
        with pytest.raises(errors.AuthError) as excinfo:
            Client.connect(srv.address, VECTORIZER, config)
        assert "unauthorized" in str(excinfo.value)


# -- Timeouts (CLT-020) ---------------------------------------------------------


def test_per_call_timeout_fires_and_late_response_is_dropped() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        slow = conn.read_request()
        # Answer nothing until the *next* request proves the timeout fired
        # client-side; then deliver the late response first.
        fresh = conn.read_request()
        conn.send_ok(slow.id, Value.str("late"))
        conn.send_ok(fresh.id, Value.str("fresh"))

    with MockServer(script) as srv:
        client = Client.connect(srv.address, plain_profile())
        with pytest.raises(errors.TimeoutError):
            client.call("SLOW", timeout=0.1)
        # The pending entry was removed (CLT-020); the late response falls
        # under the unknown-id drop (CLT-013) and the connection lives on.
        assert client.call("NEXT").as_str() == "fresh"
        assert client.unknown_response_drops() == 1
        client.close()


# -- Reconnection (CLT-030/031) --------------------------------------------------


def test_reconnect_after_server_drop_succeeds() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        request = conn.read_request()
        conn.send_ok(request.id, Value.str("first"))
        conn.close()  # connection dropped
        conn = srv.accept()
        request = conn.read_request()
        conn.send_ok(request.id, Value.str("second"))

    with MockServer(script) as srv:
        client = Client.connect(srv.address, plain_profile())
        assert client.call("A").as_str() == "first"
        # Let the reader observe the EOF and mark the connection dead.
        time.sleep(0.3)
        # CLT-030: the call finds the connection dead and lazily re-dials.
        assert client.call("B").as_str() == "second"
        client.close()


def test_reconnect_gives_up_after_two_attempts_with_typed_connection_error() -> None:
    def script(srv: MockServer) -> None:
        # Connection 1: serve the handshake and one call, then drop.
        conn = srv.accept()
        hello = conn.read_request()
        conn.send_ok(hello.id, hello_ok_reply())
        request = conn.read_request()
        conn.send_ok(request.id, Value.str("ok"))
        conn.close()
        # Re-dial attempts: accept and slam shut before the HelloMandatory
        # handshake can complete.
        for _ in range(2):
            srv.accept().close()

    with MockServer(script) as srv:
        config = ClientConfig(credentials=Credentials.api_key("k"))
        client = Client.connect(srv.address, VECTORIZER, config)
        client.call("PING")
        time.sleep(0.3)

        with pytest.raises(errors.ConnectionError):
            client.call("PING")
        assert (
            srv.accepts == 3
        ), "initial connect + exactly 2 re-dial attempts (CLT-030)"
        client.close()


# -- Error mapping (CLT-050..052) -------------------------------------------------


def test_resp3_error_mapping_over_the_wire() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        request = conn.read_request()
        conn.send_err(request.id, "NOAUTH Authentication required.")
        request = conn.read_request()
        conn.send_err(request.id, "ERR unknown command 'FOO'")

    with MockServer(script) as srv:
        client = Client.connect(srv.address, NEXUS)
        with pytest.raises(errors.AuthError) as auth_info:
            client.call("GET")
        assert auth_info.value.message == "NOAUTH Authentication required."
        with pytest.raises(errors.ServerError) as server_info:
            client.call("FOO")
        assert server_info.value.message == "ERR unknown command 'FOO'"
        assert server_info.value.code is None
        client.close()


def test_bracket_error_mapping_over_the_wire() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        hello = conn.read_request()
        conn.send_ok(hello.id, hello_ok_reply())
        request = conn.read_request()
        conn.send_err(request.id, "[collection_not_found] no such collection: docs")

    with MockServer(script) as srv:
        client = Client.connect(srv.address, VECTORIZER)
        with pytest.raises(errors.ServerError) as excinfo:
            client.call("SEARCH")
        assert (
            excinfo.value.message == "[collection_not_found] no such collection: docs"
        )
        assert excinfo.value.code == "collection_not_found"
        client.close()


# -- Push frames (CLT-060) ---------------------------------------------------------


def test_push_frames_route_to_handler_under_enabled() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        request = conn.read_request()
        # A push frame in front of the response: it must reach the handler
        # and never be matched against the pending call.
        conn.send_push(Value.str("evt"))
        conn.send_ok(request.id, Value.str("PONG"))

    with MockServer(script) as srv:
        client = Client.connect(srv.address, plain_profile(push=PushPolicy.ENABLED))
        pushed: list[Value] = []
        client.on_push(pushed.append)
        assert client.call("SUBSCRIBE").as_str() == "PONG"
        # The push preceded the response on the wire, so the reader already
        # delivered it before resolving the call.
        assert [v.as_str() for v in pushed] == ["evt"]
        assert client.unknown_response_drops() == 0
        client.close()


def test_push_frame_under_reserved_profile_poisons_connection() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        conn.read_request()
        conn.send_push(Value.null())
        conn.close()
        # The next call may reconnect (CLT-014/030): serve it.
        conn = srv.accept()
        request = conn.read_request()
        conn.send_ok(request.id, Value.str("recovered"))

    with MockServer(script) as srv:
        client = Client.connect(srv.address, plain_profile())
        # Push under Reserved is a protocol error (CLT-060).
        with pytest.raises(errors.DecodeError):
            client.call("GET")
        # Poisoned connection, lazy reconnect on the next call.
        assert client.call("GET").as_str() == "recovered"
        client.close()


# -- Poisoning (CLT-014) -------------------------------------------------------------


def test_oversized_inbound_frame_fails_typed_and_poisons() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        conn.read_request()
        # A length prefix past the profile cap — the client must refuse on
        # the prefix alone, before any body exists.
        conn.send_raw((1_000).to_bytes(4, "little"))
        conn.close()
        conn = srv.accept()
        request = conn.read_request()
        conn.send_ok(request.id, Value.str("recovered"))

    with MockServer(script) as srv:
        client = Client.connect(srv.address, plain_profile(max_frame_bytes=64))
        with pytest.raises(errors.FrameTooLargeError):
            client.call("GET")
        assert client.call("GET").as_str() == "recovered"
        client.close()


def test_malformed_frame_poisons_with_decode_error() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        conn.read_request()
        # Valid length prefix, garbage body (0xc1 is never valid MessagePack).
        conn.send_raw((4).to_bytes(4, "little") + b"\xc1\xc1\xc1\xc1")

    with MockServer(script) as srv:
        client = Client.connect(srv.address, plain_profile())
        with pytest.raises(errors.DecodeError):
            client.call("GET")
        client.close()


# -- Lifecycle (CLT-004) ----------------------------------------------------------


def test_close_is_idempotent_and_fails_in_flight_calls() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        # Swallow the request, never answer; wait out the client close.
        conn.read_request()
        with contextlib.suppress(PeerClosed):
            conn.read_request()

    with MockServer(script) as srv:
        client = Client.connect(srv.address, plain_profile())
        outcome: list[BaseException] = []

        def hang() -> None:
            try:
                client.call("HANG")
            except BaseException as exc:
                outcome.append(exc)

        pending = threading.Thread(target=hang)
        pending.start()
        time.sleep(0.15)

        client.close()
        client.close()  # idempotent (CLT-004)

        pending.join(timeout=5)
        assert not pending.is_alive()
        assert len(outcome) == 1
        assert isinstance(
            outcome[0], errors.ConnectionError
        ), "in-flight calls fail with the typed connection-closed error"
        with pytest.raises(errors.ConnectionError):
            client.call("AFTER")


# -- Endpoints (CLT-070) ------------------------------------------------------------


def test_http_url_is_rejected_at_connect() -> None:
    with pytest.raises(errors.ConnectionError) as excinfo:
        Client.connect("http://localhost:8080", plain_profile())
    message = str(excinfo.value)
    assert "RPC-only" in message and "HTTP client" in message


def test_context_manager_closes() -> None:
    def script(srv: MockServer) -> None:
        conn = srv.accept()
        request = conn.read_request()
        conn.send_ok(request.id, Value.str("PONG"))
        with contextlib.suppress(PeerClosed):
            conn.read_request()

    with MockServer(script) as srv:
        with Client.connect(srv.address, plain_profile()) as client:
            assert client.call("PING").as_str() == "PONG"
        with pytest.raises(errors.ConnectionError):
            client.call("AFTER")
