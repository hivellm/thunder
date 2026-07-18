"""Optional-TLS transport tests (SPEC-008 CAN-020, FR-29) — the Python mirror
of ``rust/thunder/tests/tls.rs``, for both the sync and the async client
(FR-28).

Three properties, each proven sync and async: an encrypted round-trip works end
to end; the plaintext path is unchanged when TLS is unused; and a cert the
client does not trust fails as a :class:`ConnectionError`, not a hang or a
crash."""

from __future__ import annotations

import contextlib

import pytest
from mockserver import SRV_CAP, MockServer, self_signed_cert

from thunder_rpc import (
    AsyncClient,
    Client,
    ClientConfig,
    ClientTls,
    Config,
    ErrorConvention,
    Handshake,
    HelloStyle,
    PushPolicy,
    TlsPolicy,
    Value,
    errors,
)

# cryptography backs the in-test self-signed cert (as trustme/rcgen would).
pytest.importorskip("cryptography")


def transport_config() -> Config:
    """A no-handshake config — these tests exercise the transport, not auth
    (mirrors the Rust ``profile()``: ``Handshake::None``)."""
    return Config(
        scheme="test",
        default_port=0,
        handshake=Handshake.NONE,
        hello_style=HelloStyle.NOT_USED,
        push=PushPolicy.RESERVED,
        max_frame_bytes=SRV_CAP,
        max_in_flight=64,
        error_codes=ErrorConvention.NONE,
        tls=TlsPolicy.OPTIONAL,
    )


@pytest.fixture
def server_cert(tmp_path):
    """A self-signed cert/key for ``localhost`` written to temp files;
    returns ``(cert_path, key_path)``."""
    cert_pem, key_pem = self_signed_cert("localhost")
    cert_path = tmp_path / "cert.pem"
    key_path = tmp_path / "key.pem"
    cert_path.write_bytes(cert_pem)
    key_path.write_bytes(key_pem)
    return str(cert_path), str(key_path)


def _echo_script(srv: MockServer) -> None:
    """PING -> PONG, ECHO -> its first arg, over one accepted connection."""
    conn = srv.accept()
    while True:
        try:
            req = conn.read_request()
        except Exception:
            return
        if req.command == "PING":
            conn.send_ok(req.id, Value.str("PONG"))
        elif req.command == "ECHO":
            arg = req.args[0] if req.args else Value.null()
            conn.send_ok(req.id, arg)
        else:
            conn.send_err(req.id, f"ERR unknown command '{req.command}'")


# -- sync --------------------------------------------------------------------


def test_tls_round_trip_encrypts_request_and_response(server_cert) -> None:
    cert_path, key_path = server_cert
    with MockServer(
        _echo_script, tls_cert_path=cert_path, tls_key_path=key_path
    ) as srv:
        # The client trusts exactly this self-signed cert and verifies the SAN
        # `localhost`.
        client = Client.connect(
            srv.address,
            transport_config(),
            ClientConfig(tls=ClientTls(server_name="localhost", ca_path=cert_path)),
        )
        assert client.call("PING").as_str() == "PONG"
        assert client.call("ECHO", [Value.str("secret-over-tls")]).as_str() == (
            "secret-over-tls"
        )
        client.close()


def test_plaintext_still_works_when_tls_is_unused() -> None:
    # Same client stack, no TLS configured — proves the default path is
    # unchanged.
    with MockServer(_echo_script) as srv:
        client = Client.connect(srv.address, transport_config())
        assert client.call("PING").as_str() == "PONG"
        client.close()


def test_cert_mismatch_is_a_connection_error(tmp_path) -> None:
    # The server presents one self-signed cert; the client trusts a DIFFERENT
    # one — verification must fail as a Connection error (FR-29).
    server_pem, server_key = self_signed_cert("localhost")
    other_pem, _ = self_signed_cert("localhost")
    cert_path = tmp_path / "cert.pem"
    key_path = tmp_path / "key.pem"
    wrong_ca = tmp_path / "wrongca.pem"
    cert_path.write_bytes(server_pem)
    key_path.write_bytes(server_key)
    wrong_ca.write_bytes(other_pem)

    def script(srv: MockServer) -> None:
        # The server-side handshake aborts when the client rejects the cert;
        # swallow it so the script exits cleanly (the client's error is what
        # the test asserts).
        with contextlib.suppress(Exception):
            srv.accept()

    with MockServer(
        script, tls_cert_path=str(cert_path), tls_key_path=str(key_path)
    ) as srv:
        with pytest.raises(errors.ConnectionError):
            Client.connect(
                srv.address,
                transport_config(),
                ClientConfig(
                    tls=ClientTls(server_name="localhost", ca_path=str(wrong_ca))
                ),
            )


# -- async -------------------------------------------------------------------


async def test_tls_round_trip_encrypts_request_and_response_async(server_cert) -> None:
    cert_path, key_path = server_cert
    with MockServer(
        _echo_script, tls_cert_path=cert_path, tls_key_path=key_path
    ) as srv:
        client = await AsyncClient.connect(
            srv.address,
            transport_config(),
            ClientConfig(tls=ClientTls(server_name="localhost", ca_path=cert_path)),
        )
        assert (await client.call("PING")).as_str() == "PONG"
        assert (await client.call("ECHO", [Value.str("secret-over-tls")])).as_str() == (
            "secret-over-tls"
        )
        await client.close()


async def test_plaintext_still_works_when_tls_is_unused_async() -> None:
    with MockServer(_echo_script) as srv:
        client = await AsyncClient.connect(srv.address, transport_config())
        assert (await client.call("PING")).as_str() == "PONG"
        await client.close()


async def test_cert_mismatch_is_a_connection_error_async(tmp_path) -> None:
    server_pem, server_key = self_signed_cert("localhost")
    other_pem, _ = self_signed_cert("localhost")
    cert_path = tmp_path / "cert.pem"
    key_path = tmp_path / "key.pem"
    wrong_ca = tmp_path / "wrongca.pem"
    cert_path.write_bytes(server_pem)
    key_path.write_bytes(server_key)
    wrong_ca.write_bytes(other_pem)

    def script(srv: MockServer) -> None:
        with contextlib.suppress(Exception):
            srv.accept()

    with MockServer(
        script, tls_cert_path=str(cert_path), tls_key_path=str(key_path)
    ) as srv:
        with pytest.raises(errors.ConnectionError):
            await AsyncClient.connect(
                srv.address,
                transport_config(),
                ClientConfig(
                    tls=ClientTls(server_name="localhost", ca_path=str(wrong_ca))
                ),
            )
