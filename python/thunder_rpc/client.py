"""The multiplexed synchronous Thunder client (SPEC-003).

One :class:`Client` owns one TCP connection (CLT-001; pooling is a layer
above, CLT-080) and demultiplexes concurrent in-flight calls over it:

- ids are monotonically increasing u32s skipping ``PUSH_ID`` (CLT-010);
- a background reader thread routes each response to its caller by id
  (CLT-010), drops unknown ids (CLT-013), and poisons the connection on
  malformed / oversized frames — every pending call fails with the same
  typed error (CLT-014);
- writes are serialized behind a lock so frames never interleave (CLT-011);
- in-flight calls are bounded by the profile's ``max_in_flight`` — excess
  calls wait, they are not refused (CLT-012);
- per-call timeouts remove the pending entry so a late response falls under
  the unknown-id drop (CLT-020);
- when a call finds the connection dead, the client lazily re-dials and
  re-handshakes up to 2 attempts with capped backoff; calls that were
  pending when the connection died fail typed and are never replayed
  (CLT-030/031);
- frames with ``id == PUSH_ID`` go to the registered push handler under
  ``PushPolicy.ENABLED`` and poison the connection under ``RESERVED``
  (CLT-060).

The demux architecture mirrors ``thunder-client``'s reader-task +
pending-map pattern; only the concurrency idiom (threads) differs (FR-28).
"""

from __future__ import annotations

import copy
import socket
import threading
import time
from typing import Callable, Iterable

from . import errors, wire
from ._handshake import (
    BACKOFF_BASE,
    BACKOFF_CAP,
    RECONNECT_ATTEMPTS,
    as_handshake_error,
    handshake_exchange,
)
from .config import ClientConfig, HandshakeInfo
from .endpoint import parse_endpoint
from .errors import from_server_message
from .profile import Profile, PushPolicy
from .value import PUSH_ID, Request, Response, Value

PushHandler = Callable[[Value], None]

_U32_WRAP = 0x1_0000_0000


def _closed_error() -> errors.ConnectionError:
    return errors.ConnectionError("client is closed")


class _WriteFailed(Exception):
    """Internal: the request never reached the wire — safe to resend on a
    fresh connection (not a replay; CLT-031 concerns frames that were
    sent)."""

    def __init__(self, error: errors.ThunderError) -> None:
        super().__init__(str(error))
        self.error = error


class _Waiter:
    """One pending call slot: the reader (or the poisoner) fills it and
    sets the event."""

    __slots__ = ("event", "response", "error")

    def __init__(self) -> None:
        self.event = threading.Event()
        self.response: Response | None = None
        self.error: errors.ThunderError | None = None


class _Gate:
    """In-flight bound (CLT-012): excess calls wait, never refused; close()
    fails waiters with the typed connection-closed error (CLT-004)."""

    def __init__(self, limit: int) -> None:
        self._limit = limit
        self._active = 0
        self._closed = False
        self._cond = threading.Condition()

    def acquire(self) -> None:
        with self._cond:
            while True:
                if self._closed:
                    raise _closed_error()
                if self._active < self._limit:
                    self._active += 1
                    return
                self._cond.wait()

    def release(self) -> None:
        with self._cond:
            self._active -= 1
            self._cond.notify()

    def close(self) -> None:
        with self._cond:
            self._closed = True
            self._cond.notify_all()


class _Conn:
    """One live connection: socket + demux state + the reader thread."""

    __slots__ = ("sock", "pending", "lock", "write_lock", "alive", "reader")

    def __init__(self, sock: socket.socket) -> None:
        self.sock = sock
        #: id -> _Waiter demux map (CLT-010); guarded by ``lock``.
        self.pending: dict[int, _Waiter] = {}
        self.lock = threading.Lock()
        #: Writes serialize behind this lock so frames never interleave
        #: (CLT-011); reads belong to the reader thread alone.
        self.write_lock = threading.Lock()
        self.alive = True
        self.reader: threading.Thread | None = None

    def poison(self, error: errors.ThunderError) -> None:
        """Mark dead and fail every pending call with the same typed error
        (CLT-014). Idempotent."""
        with self.lock:
            self.alive = False
            drained = list(self.pending.values())
            self.pending.clear()
        for waiter in drained:
            waiter.error = copy.copy(error)
            waiter.event.set()

    def kill(self, error: errors.ThunderError) -> None:
        """Tear down: fail all pending calls typed and close the socket
        (the reader unblocks and exits). Safe to call from any thread,
        including the reader itself."""
        self.poison(error)
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        try:
            self.sock.close()
        except OSError:
            pass


class Client:
    """A multiplexed, profile-driven Thunder RPC client (SPEC-003).

    Thread-safe: calls may run concurrently from any number of threads
    (CLT-010). Construct via :meth:`connect`; usable as a context manager.
    """

    def __init__(
        self, endpoint: str, profile: Profile, config: ClientConfig | None = None
    ):
        self._endpoint = parse_endpoint(endpoint)
        self._profile = profile
        self._config = config if config is not None else ClientConfig()
        self._next_id = 1
        self._id_lock = threading.Lock()
        self._gate = _Gate(profile.max_in_flight)
        self._conn: _Conn | None = None
        self._conn_lock = threading.Lock()
        #: Serializes re-dial attempts so one caller reconnects at a time.
        self._reconnect_lock = threading.Lock()
        self._closed = False
        self._push_handler: PushHandler | None = None
        self._unknown_drops = 0
        self._stats_lock = threading.Lock()
        self._handshake_info = HandshakeInfo()

    @classmethod
    def connect(
        cls, endpoint: str, profile: Profile, config: ClientConfig | None = None
    ) -> Client:
        """Dial ``endpoint`` and run the profile handshake (CLT-001/002).

        ``endpoint`` accepts every form of
        :func:`~thunder_rpc.endpoint.parse_endpoint` (CLT-070):
        ``scheme://host[:port]`` or bare ``host:port``.
        """
        client = cls(endpoint, profile, config)
        conn = client._establish()
        with client._conn_lock:
            client._conn = conn
        return client

    # -- public API -----------------------------------------------------------

    def call(
        self, command: str, args: Iterable[Value] = (), *, timeout: float | None = None
    ) -> Value:
        """Issue one call; blocks until the response, a typed error, or the
        timeout (default: the client's ``call_timeout`` — CLT-020).

        Concurrent callers multiplex over the one connection; completion
        order follows the server, not submission order (CLT-010).
        """
        if timeout is None:
            timeout = self._config.call_timeout
        args = tuple(args)
        # CLT-012: bounded in-flight — excess calls wait here, never refused.
        self._gate.acquire()
        try:
            budget = [RECONNECT_ATTEMPTS]
            while True:
                conn = self._live_conn(budget)
                try:
                    return self._dispatch(conn, command, args, timeout)
                except _WriteFailed as failure:
                    if budget[0] == 0:
                        raise failure.error from None
                    # The frame never hit the wire: reconnect and resend.
        finally:
            self._gate.release()

    def on_push(self, handler: PushHandler | None) -> None:
        """Register the push hook (CLT-060). Frames with ``id == PUSH_ID``
        are routed here under ``PushPolicy.ENABLED`` and never matched
        against pending calls. The handler runs on the reader thread — keep
        it fast and offload real work to a queue."""
        self._push_handler = handler

    def close(self) -> None:
        """Explicit, idempotent close (CLT-004): fails all in-flight calls
        with a typed connection-closed error and shuts the socket down."""
        self._closed = True
        self._gate.close()
        with self._conn_lock:
            conn = self._conn
            self._conn = None
        if conn is not None:
            conn.kill(_closed_error())
            reader = conn.reader
            if reader is not None and reader is not threading.current_thread():
                reader.join(timeout=2.0)

    def is_authenticated(self) -> bool:
        """True once the current connection's handshake authenticated
        (CLT-003 — auth is sticky per connection)."""
        return self._handshake_info.authenticated

    def capabilities(self) -> tuple[str, ...]:
        """Capabilities the server advertised in the ``HELLO`` reply."""
        return self._handshake_info.capabilities

    def handshake_info(self) -> HandshakeInfo:
        """Snapshot of what the handshake learned (CLT-002)."""
        return self._handshake_info

    def unknown_response_drops(self) -> int:
        """How many responses matched no pending call and were dropped
        (CLT-013 — client stats, never fatal)."""
        with self._stats_lock:
            return self._unknown_drops

    @property
    def profile(self) -> Profile:
        """The profile this client drives its behavior from."""
        return self._profile

    def __enter__(self) -> Client:
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    # -- internals ------------------------------------------------------------

    def _alloc_id(self) -> int:
        """Allocate the next request id, skipping ``PUSH_ID`` (CLT-010)."""
        with self._id_lock:
            while True:
                frame_id = self._next_id
                self._next_id = (self._next_id + 1) % _U32_WRAP
                if frame_id != PUSH_ID:
                    return frame_id

    def _live_conn(self, budget: list[int]) -> _Conn:
        """Return the current live connection, lazily reconnecting when it
        is dead or absent: up to ``budget`` re-dial + re-handshake attempts
        with capped backoff (CLT-030). Never replays in-flight calls — those
        already failed typed when the connection died (CLT-031)."""
        if self._closed:
            raise _closed_error()
        with self._conn_lock:
            conn = self._conn
        if conn is not None and conn.alive:
            return conn
        with self._reconnect_lock:
            if self._closed:
                raise _closed_error()
            # Another caller may have reconnected while we waited.
            with self._conn_lock:
                conn = self._conn
            if conn is not None and conn.alive:
                return conn
            last_error: errors.ThunderError = errors.ConnectionError(
                "connection is dead"
            )
            backoff = BACKOFF_BASE
            while budget[0] > 0:
                budget[0] -= 1
                try:
                    conn = self._establish()
                except errors.AuthError:
                    # An auth rejection is deterministic — retrying cannot fix it.
                    raise
                except errors.ThunderError as exc:
                    last_error = exc
                    if budget[0] > 0:
                        time.sleep(backoff)
                        backoff = min(backoff * 2, BACKOFF_CAP)
                else:
                    with self._conn_lock:
                        self._conn = conn
                    return conn
            raise last_error

    def _establish(self) -> _Conn:
        """Dial (with the connect timeout, TCP_NODELAY on — CLT-001), start
        the reader thread, and run the profile handshake (CLT-002)."""
        host, port = self._endpoint.host, self._endpoint.port
        try:
            sock = socket.create_connection(
                (host, port), timeout=self._config.connect_timeout
            )
        except TimeoutError as exc:
            raise errors.TimeoutError() from exc
        except OSError as exc:
            raise errors.ConnectionError(
                f"connect to {host}:{port} failed: {exc}"
            ) from exc
        try:
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            sock.settimeout(None)
        except OSError as exc:
            sock.close()
            raise errors.ConnectionError(f"TCP_NODELAY failed: {exc}") from exc
        conn = _Conn(sock)
        reader = threading.Thread(
            target=self._reader_loop, args=(conn,), name="thunder-reader", daemon=True
        )
        conn.reader = reader
        reader.start()
        try:
            info = self._run_handshake(conn)
        except BaseException:
            # Failure tears the connection down; the caller sees the typed error.
            conn.kill(errors.ConnectionError("handshake failed"))
            raise
        self._handshake_info = info
        return conn

    def _run_handshake(self, conn: _Conn) -> HandshakeInfo:
        exchange = handshake_exchange(self._profile, self._config)
        reply: Value | None = None
        while True:
            try:
                command, args = exchange.send(reply)
            except StopIteration as stop:
                return stop.value if stop.value is not None else HandshakeInfo()
            reply = self._handshake_call(conn, command, args)

    def _handshake_call(
        self, conn: _Conn, command: str, args: tuple[Value, ...]
    ) -> Value:
        """One handshake round-trip. Server rejections surface as the typed
        auth class, never a generic error (CLT-003)."""
        try:
            return self._dispatch(conn, command, args, self._config.call_timeout)
        except _WriteFailed as failure:
            raise as_handshake_error(failure.error) from None
        except errors.ThunderError as exc:
            raise as_handshake_error(exc) from None

    def _dispatch(
        self, conn: _Conn, command: str, args: tuple[Value, ...], timeout: float
    ) -> Value:
        """One request/response attempt on one connection: register the
        pending entry, write the frame (serialized, CLT-011), await the
        demuxed response under the timeout (CLT-020)."""
        frame_id = self._alloc_id()
        request = Request(id=frame_id, command=command, args=args)
        frame = wire.encode_frame(
            request, max_frame_bytes=self._profile.max_frame_bytes
        )
        waiter = _Waiter()
        with conn.lock:
            # Register while checking liveness in the same critical section
            # the poisoner drains under — a dying connection either fails
            # this entry or is seen dead.
            if not conn.alive:
                raise _WriteFailed(errors.ConnectionError("connection is dead"))
            conn.pending[frame_id] = waiter
        try:
            with conn.write_lock:
                conn.sock.sendall(frame)
        except OSError as exc:
            with conn.lock:
                conn.pending.pop(frame_id, None)
            error = errors.ConnectionError(f"write failed: {exc}")
            conn.kill(error)
            raise _WriteFailed(error) from None
        if not waiter.event.wait(timeout):
            # CLT-020: remove the pending entry on timeout; a late response
            # to this id is dropped per CLT-013.
            with conn.lock:
                conn.pending.pop(frame_id, None)
            raise errors.TimeoutError()
        if waiter.error is not None:
            raise waiter.error
        response = waiter.response
        assert response is not None
        if response.err is not None:
            raise from_server_message(response.err, self._profile.error_codes)
        return response.ok

    def _reader_loop(self, conn: _Conn) -> None:
        """The background reader (CLT-010): reads frames with the profile
        cap, demuxes by id, routes push frames (CLT-060), drops unknown ids
        (CLT-013), and poisons the connection on any failure (CLT-014)."""
        cap = self._profile.max_frame_bytes
        while True:
            try:
                header = _read_exact(conn.sock, 4)
                length = int.from_bytes(header, "little")
                if length > cap:
                    # WIRE-020/021: refuse from the prefix alone, before the
                    # body is read or allocated.
                    raise errors.frame_too_large(length, cap)
                body = _read_exact(conn.sock, length)
                response = wire.decode_response_body(body)
            except errors.ThunderError as exc:
                error = exc
                break
            except OSError as exc:
                error = errors.ConnectionError(f"connection lost: {exc}")
                break
            if response.id == PUSH_ID:
                if self._profile.push is PushPolicy.ENABLED:
                    handler = self._push_handler
                    if handler is not None and response.err is None:
                        try:
                            handler(response.ok)
                        except Exception:
                            pass  # a broken hook must not take the reader down
                    continue
                # Protocol error under Reserved profiles: poison per CLT-014.
                error = errors.DecodeError(
                    "server sent a push frame but the profile reserves PUSH_ID (CLT-060)"
                )
                break
            with conn.lock:
                waiter = conn.pending.pop(response.id, None)
            if waiter is not None:
                waiter.response = response
                waiter.event.set()
            else:
                # CLT-013: unknown id — count and drop, never fatal.
                with self._stats_lock:
                    self._unknown_drops += 1
        # CLT-014: fail all pending calls typed and close our side.
        conn.kill(error)


def _read_exact(sock: socket.socket, size: int) -> bytes:
    if size == 0:
        return b""
    buf = bytearray(size)
    view = memoryview(buf)
    got = 0
    while got < size:
        read = sock.recv_into(view[got:], size - got)
        if read == 0:
            raise errors.ConnectionError("connection closed by peer")
        got += read
    return bytes(buf)
