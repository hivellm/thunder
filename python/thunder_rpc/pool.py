"""Optional connection pool (CLT-080) — a layer **above** the
single-connection clients (CLT-001: "pooling is a layer above").

Under a mandatory-``HELLO`` profile with ``auth_required``, a fresh connection
costs a handshake round trip before the first request. A caller that opens a
connection per operation therefore pays that round trip every time. The pool
amortizes it: ``N`` operations over a checked-out connection pay **one**
connect and **one** handshake, not ``N``.

The shape is deliberately minimal — a fixed number of connections bounded by a
semaphore, an idle list, lazy connect on first checkout, and a guard that
returns the connection to the pool on exit. It is **not** an external pool
library. Health checks, background reaping and min-idle warmup are out of
scope; a poisoned connection (CLT-014) is dropped on return and the next
checkout connects fresh, leaving reconnect to CLT-030 rather than the pool.

The pool adds **no wire behavior**: it builds the same client as
``Client.connect`` / ``AsyncClient.connect`` from a :class:`Config` and a
:class:`ClientConfig`. ``max_in_flight`` (CLT-012) stays a per-connection
bound; the pool bounds connections, not in-flight calls.

Python idiom: the guard is a context manager (FR-28) — ``with pool.acquire()
as client:`` for the sync :class:`Pool`, ``async with pool.acquire() as
client:`` for the async :class:`AsyncPool` — releasing the connection on exit,
including the error path.

Example::

    from thunder_rpc import ClientConfig, Config, Pool

    app = Config.standard().with_scheme("myapp").with_port(9000)
    pool = Pool("myapp://localhost", app, ClientConfig(), max_connections=8)
    with pool.acquire() as client:      # reuses an idle connection, or dials one
        pong = client.call("PING")
        assert pong.as_str() == "PONG"
    # the connection returns to the pool when the `with` block exits.
"""

from __future__ import annotations

import asyncio
import threading

from .aio import AsyncClient
from .client import Client
from .client_config import ClientConfig
from .config import Config


class Pool:
    """A bounded pool of synchronous :class:`~thunder_rpc.client.Client`\\ s
    over one endpoint (CLT-080).

    At most ``max_connections`` connections are live at once; a checkout beyond
    that awaits a return. Connections are dialed lazily — construction opens
    none — and reused across checkouts so the handshake is paid once per
    connection, not once per operation. ``max_connections`` is clamped to at
    least 1.
    """

    def __init__(
        self,
        endpoint: str,
        config: Config,
        client_config: ClientConfig | None = None,
        max_connections: int = 8,
    ) -> None:
        self._endpoint = endpoint
        self._config = config
        self._client_config = (
            client_config if client_config is not None else ClientConfig()
        )
        max_conn = max(1, max_connections)
        #: Bounds live + checked-out connections to ``max_connections``.
        self._permits = threading.BoundedSemaphore(max_conn)
        self._idle: list[Client] = []
        self._idle_lock = threading.Lock()

    def acquire(self) -> _PooledConn:
        """Return a guard; entering its ``with`` block checks out a connection
        (reusing an idle, live one or dialing a fresh one) and yields the
        client, returning it to the pool on exit (CLT-080)."""
        return _PooledConn(self)

    def idle_count(self) -> int:
        """Idle connections currently parked in the pool. For diagnostics and
        tests — production code should not branch on it."""
        with self._idle_lock:
            return len(self._idle)

    def _checkout(self) -> Client:
        # Awaits a return when max_connections are already checked out.
        self._permits.acquire()
        try:
            # Reuse the newest idle connection that is still live; discard any
            # poisoned (CLT-014) while sitting idle.
            reused: Client | None = None
            with self._idle_lock:
                while self._idle:
                    candidate = self._idle.pop()
                    if candidate.is_alive():
                        reused = candidate
                        break
                    candidate.close()
            if reused is not None:
                return reused
            return Client.connect(self._endpoint, self._config, self._client_config)
        except BaseException:
            self._permits.release()
            raise

    def _checkin(self, client: Client) -> None:
        # CLT-014: only a live connection returns to the pool; a poisoned or
        # closed one is dropped here, and the next checkout dials fresh.
        try:
            if client.is_alive():
                with self._idle_lock:
                    self._idle.append(client)
            else:
                client.close()
        finally:
            self._permits.release()


class _PooledConn:
    """Guard from :meth:`Pool.acquire`. A context manager: entering checks out
    a client, exiting returns it to the pool (or drops it if poisoned,
    CLT-014). ``__exit__`` runs on the error path too, so a connection never
    leaks."""

    __slots__ = ("_pool", "_client")

    def __init__(self, pool: Pool) -> None:
        self._pool = pool
        self._client: Client | None = None

    def __enter__(self) -> Client:
        self._client = self._pool._checkout()
        return self._client

    def __exit__(self, *_exc: object) -> None:
        client = self._client
        self._client = None
        if client is not None:
            self._pool._checkin(client)


class AsyncPool:
    """A bounded pool of :class:`~thunder_rpc.aio.AsyncClient`\\ s over one
    endpoint (CLT-080) — the asyncio mirror of :class:`Pool`, identical
    semantics, ``async with`` idiom only (FR-28)."""

    def __init__(
        self,
        endpoint: str,
        config: Config,
        client_config: ClientConfig | None = None,
        max_connections: int = 8,
    ) -> None:
        self._endpoint = endpoint
        self._config = config
        self._client_config = (
            client_config if client_config is not None else ClientConfig()
        )
        self._max = max(1, max_connections)
        self._permits: asyncio.BoundedSemaphore | None = None
        self._idle: list[AsyncClient] = []

    def acquire(self) -> _AsyncPooledConn:
        """Return a guard; entering its ``async with`` block checks out a
        connection (reusing an idle, live one or dialing a fresh one) and
        yields the client, returning it to the pool on exit (CLT-080)."""
        return _AsyncPooledConn(self)

    def idle_count(self) -> int:
        """Idle connections currently parked in the pool. For diagnostics and
        tests — production code should not branch on it."""
        return len(self._idle)

    def _semaphore(self) -> asyncio.BoundedSemaphore:
        # Created lazily so the pool can be constructed off the event loop.
        if self._permits is None:
            self._permits = asyncio.BoundedSemaphore(self._max)
        return self._permits

    async def _checkout(self) -> AsyncClient:
        permits = self._semaphore()
        # Awaits a return when max_connections are already checked out.
        await permits.acquire()
        try:
            # Reuse the newest idle, live connection; discard poisoned ones.
            reused: AsyncClient | None = None
            while self._idle:
                candidate = self._idle.pop()
                if candidate.is_alive():
                    reused = candidate
                    break
                await candidate.close()
            if reused is not None:
                return reused
            return await AsyncClient.connect(
                self._endpoint, self._config, self._client_config
            )
        except BaseException:
            permits.release()
            raise

    async def _checkin(self, client: AsyncClient) -> None:
        # CLT-014: only a live connection returns to the pool; a poisoned or
        # closed one is dropped here, and the next checkout dials fresh.
        try:
            if client.is_alive():
                self._idle.append(client)
            else:
                await client.close()
        finally:
            self._semaphore().release()


class _AsyncPooledConn:
    """Guard from :meth:`AsyncPool.acquire`. An async context manager:
    entering checks out a client, exiting returns it to the pool (or drops it
    if poisoned, CLT-014). ``__aexit__`` runs on the error path too."""

    __slots__ = ("_pool", "_client")

    def __init__(self, pool: AsyncPool) -> None:
        self._pool = pool
        self._client: AsyncClient | None = None

    async def __aenter__(self) -> AsyncClient:
        self._client = await self._pool._checkout()
        return self._client

    async def __aexit__(self, *_exc: object) -> None:
        client = self._client
        self._client = None
        if client is not None:
            await self._pool._checkin(client)


__all__ = ["AsyncPool", "Pool"]
