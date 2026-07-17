"""The multiplexed asyncio Thunder client (SPEC-003).

The contract is IDENTICAL to :class:`thunder_rpc.client.Client` — it differs
in idiom only (FR-28): a reader task instead of a reader thread, futures
instead of events, ``asyncio.Lock`` write serialization. Codec, profile,
endpoint, error, and handshake semantics are the same shared modules.

asyncio extra (CLT-021): cancelling a ``call()`` removes its pending entry,
so a late response to that id falls under the unknown-id drop (CLT-013).
"""

from __future__ import annotations

import asyncio
import copy
import inspect
import socket
from collections import deque
from typing import Any, Callable, Iterable

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
from .value import PUSH_ID, Request, Value

PushHandler = Callable[[Value], Any]

_U32_WRAP = 0x1_0000_0000


def _closed_error() -> errors.ConnectionError:
    return errors.ConnectionError("client is closed")


class _WriteFailed(Exception):
    """Internal: the request never reached the wire — safe to resend on a
    fresh connection (CLT-031 concerns frames that were sent)."""

    def __init__(self, error: errors.ThunderError) -> None:
        super().__init__(str(error))
        self.error = error


class _AsyncGate:
    """In-flight bound (CLT-012): excess calls wait, never refused; close()
    fails waiters with the typed connection-closed error (CLT-004)."""

    def __init__(self, limit: int) -> None:
        self._limit = limit
        self._active = 0
        self._closed = False
        self._waiters: deque[asyncio.Future] = deque()

    async def acquire(self) -> None:
        while True:
            if self._closed:
                raise _closed_error()
            if self._active < self._limit:
                self._active += 1
                return
            fut = asyncio.get_running_loop().create_future()
            self._waiters.append(fut)
            try:
                await fut
            except asyncio.CancelledError:
                try:
                    self._waiters.remove(fut)
                except ValueError:
                    pass
                raise

    def release(self) -> None:
        self._active -= 1
        while self._waiters:
            fut = self._waiters.popleft()
            if not fut.done():
                fut.set_result(None)
                return

    def close(self) -> None:
        self._closed = True
        while self._waiters:
            fut = self._waiters.popleft()
            if not fut.done():
                fut.set_result(None)  # woken waiters re-check and raise


class _AsyncConn:
    """One live connection: stream pair + demux state + the reader task."""

    __slots__ = ("reader", "writer", "pending", "write_lock", "alive", "reader_task")

    def __init__(
        self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter
    ) -> None:
        self.reader = reader
        self.writer = writer
        #: id -> Future demux map (CLT-010).
        self.pending: dict[int, asyncio.Future] = {}
        #: Writes serialize behind this lock so frames never interleave
        #: (CLT-011); reads belong to the reader task alone.
        self.write_lock = asyncio.Lock()
        self.alive = True
        self.reader_task: asyncio.Task | None = None

    def poison(self, error: errors.ThunderError) -> None:
        """Mark dead and fail every pending call with the same typed error
        (CLT-014). Idempotent."""
        self.alive = False
        drained = list(self.pending.values())
        self.pending.clear()
        for fut in drained:
            if not fut.done():
                fut.set_exception(copy.copy(error))

    def kill(self, error: errors.ThunderError) -> None:
        """Tear down: fail all pending calls typed, stop the reader, close
        the transport. Safe to call from the reader task itself."""
        self.poison(error)
        task = self.reader_task
        if task is not None and not task.done() and task is not asyncio.current_task():
            task.cancel()
        try:
            self.writer.close()
        except Exception:
            pass


class AsyncClient:
    """A multiplexed, profile-driven Thunder RPC client for asyncio
    (SPEC-003). Construct via ``await AsyncClient.connect(...)``; usable as
    an async context manager."""

    def __init__(
        self, endpoint: str, profile: Profile, config: ClientConfig | None = None
    ):
        self._endpoint = parse_endpoint(endpoint)
        self._profile = profile
        self._config = config if config is not None else ClientConfig()
        self._next_id = 1
        self._gate = _AsyncGate(profile.max_in_flight)
        self._conn: _AsyncConn | None = None
        #: Serializes re-dial attempts so one caller reconnects at a time.
        self._reconnect_lock = asyncio.Lock()
        self._closed = False
        self._push_handler: PushHandler | None = None
        self._unknown_drops = 0
        self._handshake_info = HandshakeInfo()

    @classmethod
    async def connect(
        cls, endpoint: str, profile: Profile, config: ClientConfig | None = None
    ) -> AsyncClient:
        """Dial ``endpoint`` and run the profile handshake (CLT-001/002).

        ``endpoint`` accepts every form of
        :func:`~thunder_rpc.endpoint.parse_endpoint` (CLT-070):
        ``scheme://host[:port]`` or bare ``host:port``.
        """
        client = cls(endpoint, profile, config)
        client._conn = await client._establish()
        return client

    # -- public API -----------------------------------------------------------

    async def call(
        self, command: str, args: Iterable[Value] = (), *, timeout: float | None = None
    ) -> Value:
        """Issue one call (CLT-020 timeout; default the client's
        ``call_timeout``). Concurrent callers multiplex over the one
        connection; completion order follows the server (CLT-010).
        Cancellation removes the pending entry (CLT-021)."""
        if timeout is None:
            timeout = self._config.call_timeout
        args = tuple(args)
        # CLT-012: bounded in-flight — excess calls wait here, never refused.
        await self._gate.acquire()
        try:
            budget = [RECONNECT_ATTEMPTS]
            while True:
                conn = await self._live_conn(budget)
                try:
                    return await self._dispatch(conn, command, args, timeout)
                except _WriteFailed as failure:
                    if budget[0] == 0:
                        raise failure.error from None
                    # The frame never hit the wire: reconnect and resend.
        finally:
            self._gate.release()

    def on_push(self, handler: PushHandler | None) -> None:
        """Register the push hook (CLT-060). Frames with ``id == PUSH_ID``
        are routed here under ``PushPolicy.ENABLED`` and never matched
        against pending calls. The handler runs on the reader task; a
        returned awaitable is scheduled as a task."""
        self._push_handler = handler

    async def close(self) -> None:
        """Explicit, idempotent close (CLT-004): fails all in-flight calls
        with a typed connection-closed error and shuts the transport down."""
        self._closed = True
        self._gate.close()
        conn = self._conn
        self._conn = None
        if conn is not None:
            conn.kill(_closed_error())
            task = conn.reader_task
            if task is not None:
                try:
                    await task
                except (asyncio.CancelledError, Exception):
                    pass
            try:
                await conn.writer.wait_closed()
            except Exception:
                pass

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
        return self._unknown_drops

    @property
    def profile(self) -> Profile:
        """The profile this client drives its behavior from."""
        return self._profile

    async def __aenter__(self) -> AsyncClient:
        return self

    async def __aexit__(self, *_exc: object) -> None:
        await self.close()

    # -- internals ------------------------------------------------------------

    def _alloc_id(self) -> int:
        """Allocate the next request id, skipping ``PUSH_ID`` (CLT-010)."""
        while True:
            frame_id = self._next_id
            self._next_id = (self._next_id + 1) % _U32_WRAP
            if frame_id != PUSH_ID:
                return frame_id

    async def _live_conn(self, budget: list[int]) -> _AsyncConn:
        """Return the current live connection, lazily reconnecting when it
        is dead or absent: up to ``budget`` re-dial + re-handshake attempts
        with capped backoff (CLT-030). Never replays in-flight calls
        (CLT-031)."""
        if self._closed:
            raise _closed_error()
        conn = self._conn
        if conn is not None and conn.alive:
            return conn
        async with self._reconnect_lock:
            if self._closed:
                raise _closed_error()
            # Another caller may have reconnected while we waited.
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
                    conn = await self._establish()
                except errors.AuthError:
                    # An auth rejection is deterministic — retrying cannot fix it.
                    raise
                except errors.ThunderError as exc:
                    last_error = exc
                    if budget[0] > 0:
                        await asyncio.sleep(backoff)
                        backoff = min(backoff * 2, BACKOFF_CAP)
                else:
                    self._conn = conn
                    return conn
            raise last_error

    async def _establish(self) -> _AsyncConn:
        """Dial (with the connect timeout, TCP_NODELAY on — CLT-001), start
        the reader task, and run the profile handshake (CLT-002)."""
        host, port = self._endpoint.host, self._endpoint.port
        try:
            reader, writer = await asyncio.wait_for(
                asyncio.open_connection(host, port), self._config.connect_timeout
            )
        except asyncio.TimeoutError as exc:
            raise errors.TimeoutError() from exc
        except OSError as exc:
            raise errors.ConnectionError(
                f"connect to {host}:{port} failed: {exc}"
            ) from exc
        sock = writer.get_extra_info("socket")
        if sock is not None:
            try:
                sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            except OSError as exc:
                writer.close()
                raise errors.ConnectionError(f"TCP_NODELAY failed: {exc}") from exc
        conn = _AsyncConn(reader, writer)
        conn.reader_task = asyncio.get_running_loop().create_task(
            self._reader_loop(conn)
        )
        try:
            info = await self._run_handshake(conn)
        except BaseException:
            # Failure tears the connection down; the caller sees the typed error.
            conn.kill(errors.ConnectionError("handshake failed"))
            raise
        self._handshake_info = info
        return conn

    async def _run_handshake(self, conn: _AsyncConn) -> HandshakeInfo:
        exchange = handshake_exchange(self._profile, self._config)
        reply: Value | None = None
        while True:
            try:
                command, args = exchange.send(reply)
            except StopIteration as stop:
                return stop.value if stop.value is not None else HandshakeInfo()
            reply = await self._handshake_call(conn, command, args)

    async def _handshake_call(
        self, conn: _AsyncConn, command: str, args: tuple[Value, ...]
    ) -> Value:
        """One handshake round-trip. Server rejections surface as the typed
        auth class, never a generic error (CLT-003)."""
        try:
            return await self._dispatch(conn, command, args, self._config.call_timeout)
        except _WriteFailed as failure:
            raise as_handshake_error(failure.error) from None
        except errors.ThunderError as exc:
            raise as_handshake_error(exc) from None

    async def _dispatch(
        self, conn: _AsyncConn, command: str, args: tuple[Value, ...], timeout: float
    ) -> Value:
        """One request/response attempt on one connection: register the
        pending entry, write the frame (serialized, CLT-011), await the
        demuxed response under the timeout (CLT-020)."""
        frame_id = self._alloc_id()
        request = Request(id=frame_id, command=command, args=args)
        frame = wire.encode_frame(
            request, max_frame_bytes=self._profile.max_frame_bytes
        )
        if not conn.alive:
            raise _WriteFailed(errors.ConnectionError("connection is dead"))
        fut = asyncio.get_running_loop().create_future()
        conn.pending[frame_id] = fut
        try:
            async with conn.write_lock:
                conn.writer.write(frame)
                await conn.writer.drain()
        except OSError as exc:
            conn.pending.pop(frame_id, None)
            error = errors.ConnectionError(f"write failed: {exc}")
            conn.kill(error)
            raise _WriteFailed(error) from None
        try:
            response = await asyncio.wait_for(fut, timeout)
        except asyncio.TimeoutError:
            # CLT-020: remove the pending entry on timeout; a late response
            # to this id is dropped per CLT-013.
            conn.pending.pop(frame_id, None)
            raise errors.TimeoutError() from None
        except asyncio.CancelledError:
            # CLT-021: cancellation removes the pending entry.
            conn.pending.pop(frame_id, None)
            raise
        if response.err is not None:
            raise from_server_message(response.err, self._profile.error_codes)
        return response.ok

    async def _reader_loop(self, conn: _AsyncConn) -> None:
        """The background reader (CLT-010): reads frames with the profile
        cap, demuxes by id, routes push frames (CLT-060), drops unknown ids
        (CLT-013), and poisons the connection on any failure (CLT-014)."""
        cap = self._profile.max_frame_bytes
        while True:
            try:
                header = await conn.reader.readexactly(4)
                length = int.from_bytes(header, "little")
                if length > cap:
                    # WIRE-020/021: refuse from the prefix alone, before the
                    # body is read or allocated.
                    raise errors.frame_too_large(length, cap)
                body = await conn.reader.readexactly(length) if length else b""
                response = wire.decode_response_body(body)
            except errors.ThunderError as exc:
                error = exc
                break
            except asyncio.IncompleteReadError:
                error = errors.ConnectionError("connection closed by peer")
                break
            except OSError as exc:
                error = errors.ConnectionError(f"connection lost: {exc}")
                break
            except asyncio.CancelledError:
                return  # killed externally; the killer already poisoned
            if response.id == PUSH_ID:
                if self._profile.push is PushPolicy.ENABLED:
                    handler = self._push_handler
                    if handler is not None and response.err is None:
                        try:
                            result = handler(response.ok)
                            if inspect.isawaitable(result):
                                asyncio.ensure_future(result)
                        except Exception:
                            pass  # a broken hook must not take the reader down
                    continue
                # Protocol error under Reserved profiles: poison per CLT-014.
                error = errors.DecodeError(
                    "server sent a push frame but the profile reserves PUSH_ID (CLT-060)"
                )
                break
            fut = conn.pending.pop(response.id, None)
            if fut is not None:
                if not fut.done():
                    fut.set_result(response)
            else:
                # CLT-013: unknown id — count and drop, never fatal.
                self._unknown_drops += 1
        # CLT-014: fail all pending calls typed and close our side.
        conn.kill(error)
