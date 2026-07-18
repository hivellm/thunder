"""Scripted loopback TCP responders for the behavioral floor tests
(SPEC-003 / CLT-090) — the Python mirror of the tokio responders in
``rust/thunder/tests/behavior.rs``. Built on the thunder_rpc wire
codec; serves both the sync and the asyncio client (the server side is
always a plain thread)."""

from __future__ import annotations

import datetime
import socket
import ssl
import threading
from typing import Callable

from thunder_rpc import PUSH_ID, Request, Response, Value, wire


def self_signed_cert(common_name: str = "localhost") -> tuple[bytes, bytes]:
    """A fresh self-signed cert/key for ``common_name``, PEM-encoded — the
    Python mirror of the Rust ``rcgen`` helper in ``tests/tls.rs``. Generated
    in-test (no fixtures checked in) via ``cryptography``. The cert carries a
    matching SAN and ``CA:TRUE`` so it can act as its own trust anchor when the
    client pins it via ``ca_path`` (the self-signed pattern)."""
    from cryptography import x509
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import rsa
    from cryptography.x509.oid import NameOID

    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, common_name)])
    now = datetime.datetime.now(datetime.timezone.utc)
    cert = (
        x509.CertificateBuilder()
        .subject_name(name)
        .issuer_name(name)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(days=1))
        .not_valid_after(now + datetime.timedelta(days=3650))
        .add_extension(
            x509.SubjectAlternativeName([x509.DNSName(common_name)]), critical=False
        )
        .add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
        .sign(key, hashes.SHA256())
    )
    cert_pem = cert.public_bytes(serialization.Encoding.PEM)
    key_pem = key.private_bytes(
        serialization.Encoding.PEM,
        serialization.PrivateFormat.TraditionalOpenSSL,
        serialization.NoEncryption(),
    )
    return cert_pem, key_pem


#: Frame cap the loopback responders read with.
SRV_CAP = 1024 * 1024

#: Generous safety timeout so a broken test fails instead of hanging.
IO_TIMEOUT = 10.0


class PeerClosed(Exception):
    """The client closed the connection while the script was reading."""


class ServerConn:
    """One accepted connection, with frame-level helpers."""

    def __init__(self, sock: socket.socket) -> None:
        self.sock = sock
        sock.settimeout(IO_TIMEOUT)

    def read_request(self) -> Request:
        header = self._read_exact(4)
        length = int.from_bytes(header, "little")
        assert length <= SRV_CAP, f"client sent an over-cap frame ({length} bytes)"
        body = self._read_exact(length)
        return wire.decode_request_body(body)

    def send_ok(self, frame_id: int, value: Value) -> None:
        self.send_raw(wire.encode_frame(Response(id=frame_id, ok=value)))

    def send_err(self, frame_id: int, message: str) -> None:
        self.send_raw(wire.encode_frame(Response(id=frame_id, err=message)))

    def send_push(self, value: Value) -> None:
        self.send_raw(wire.encode_frame(Response(id=PUSH_ID, ok=value)))

    def send_raw(self, data: bytes) -> None:
        self.sock.sendall(data)

    def close(self) -> None:
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        try:
            self.sock.close()
        except OSError:
            pass

    def _read_exact(self, size: int) -> bytes:
        buf = bytearray(size)
        view = memoryview(buf)
        got = 0
        while got < size:
            read = self.sock.recv_into(view[got:], size - got)
            if read == 0:
                raise PeerClosed()
            got += read
        return bytes(buf)


class MockServer:
    """Runs ``script(server)`` on a background thread; the script drives
    ``accept()`` / frame helpers. Use as a context manager — exit joins the
    script and re-raises anything it tripped on."""

    def __init__(
        self,
        script: Callable[["MockServer"], None],
        *,
        tls_cert_path: str | None = None,
        tls_key_path: str | None = None,
    ) -> None:
        self._script = script
        #: When set, accepted sockets are wrapped in TLS before framing —
        #: the mirror of the tokio ``TlsAcceptor`` path (SRV-040). Off by
        #: default, so the plaintext responders are unchanged.
        self._server_ctx: ssl.SSLContext | None = None
        if tls_cert_path is not None and tls_key_path is not None:
            self._server_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            self._server_ctx.load_cert_chain(
                certfile=tls_cert_path, keyfile=tls_key_path
            )
        self._listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._listener.bind(("127.0.0.1", 0))
        self._listener.listen(8)
        self._listener.settimeout(IO_TIMEOUT)
        self.port = self._listener.getsockname()[1]
        #: Endpoint string the clients dial (bare host:port, CLT-070).
        self.address = f"127.0.0.1:{self.port}"
        #: How many connections the script accepted (reconnect assertions).
        self.accepts = 0
        self._conns: list[ServerConn] = []
        self._error: BaseException | None = None
        self._thread = threading.Thread(
            target=self._run, name="mock-server", daemon=True
        )

    def accept(self) -> ServerConn:
        sock, _ = self._listener.accept()
        self.accepts += 1
        # A socket returned by accept() is in blocking mode regardless of the
        # listener's timeout, so it must be given one BEFORE the TLS handshake
        # below — otherwise a handshake that never completes parks this thread
        # forever and the client just sits out its own 30 s call timeout. That
        # is exactly how this surfaced: a CI run where the server was wedged in
        # wrap_socket and the test failed as a client-side TimeoutError,
        # pointing at the wrong end.
        sock.settimeout(IO_TIMEOUT)
        if self._server_ctx is not None:
            # The TLS handshake completes here, before any Thunder frame — no
            # STARTTLS. A mismatched/untrusted client aborts it; the script is
            # expected to tolerate that (see the cert-mismatch tests).
            sock = self._server_ctx.wrap_socket(sock, server_side=True)
        conn = ServerConn(sock)
        self._conns.append(conn)
        return conn

    def _run(self) -> None:
        try:
            self._script(self)
        except BaseException as exc:  # surface script failures on __exit__
            self._error = exc

    def __enter__(self) -> "MockServer":
        self._thread.start()
        return self

    def __exit__(self, exc_type: object, *_exc: object) -> None:
        self._thread.join(timeout=IO_TIMEOUT + 5)
        for conn in self._conns:
            conn.close()
        self._listener.close()
        if exc_type is None:
            assert not self._thread.is_alive(), "mock server script did not finish"
            if self._error is not None:
                raise self._error
