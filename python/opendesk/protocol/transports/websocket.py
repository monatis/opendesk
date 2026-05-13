"""WebSocket transport for the opendesk wire protocol.

Wraps the ``websockets`` library so each opendesk protocol frame is one
WebSocket binary message.  Text frames are rejected — we are a strictly
binary protocol.

Client usage::

    from opendesk.protocol.transports.websocket import connect_websocket
    from opendesk.computer.remote import RemoteComputer

    conn = await connect_websocket("ws://mac-mini.local:8423")
    remote = await RemoteComputer.connect(conn)

Server usage::

    from opendesk.protocol.transports.websocket import serve_websocket

    async def handle(conn):
        # build a Peer / dispatcher around `conn` and run the session
        ...

    async with serve_websocket(handle, host="0.0.0.0", port=8423) as server:
        await server.wait_closed()
"""

from __future__ import annotations

import asyncio
import contextlib
from typing import Awaitable, Callable, Optional

try:
    import websockets
    from websockets.asyncio.client import ClientConnection
    from websockets.asyncio.server import Server, ServerConnection
    from websockets.exceptions import ConnectionClosed as _WSClosed
except ImportError as _exc:  # pragma: no cover
    raise ImportError(
        "websockets is required for the WebSocket transport. "
        "Install with: pip install 'opendesk[remote]'"
    ) from _exc

from opendesk.protocol.connection import Connection, ConnectionClosed


# A connection can be either a server-side ``ServerConnection`` or a
# client-side ``ClientConnection`` — both expose the same send/recv/close API.
_WSConnection = "ServerConnection | ClientConnection"


class WebSocketConnection(Connection):
    """One opendesk :class:`Connection` backed by a single WebSocket link."""

    def __init__(self, ws) -> None:
        self._ws = ws
        self._closed = False

    async def send(self, data: bytes) -> None:
        if self._closed:
            raise ConnectionClosed("send on closed connection")
        try:
            await self._ws.send(data)
        except _WSClosed as exc:
            self._closed = True
            raise ConnectionClosed(str(exc)) from exc

    async def recv(self) -> bytes:
        if self._closed:
            raise ConnectionClosed("recv on closed connection")
        try:
            msg = await self._ws.recv()
        except _WSClosed as exc:
            self._closed = True
            raise ConnectionClosed(str(exc)) from exc
        if isinstance(msg, str):
            await self.aclose()
            raise ConnectionClosed(
                "received text frame; opendesk protocol is binary-only"
            )
        return msg

    async def aclose(self) -> None:
        if self._closed:
            return
        self._closed = True
        with contextlib.suppress(Exception):
            await self._ws.close()


# ---------------------------------------------------------------------------
# Client
# ---------------------------------------------------------------------------


async def connect_websocket(
    url: str,
    *,
    ssl=None,
    additional_headers: Optional[dict] = None,
    max_message_size: Optional[int] = None,
) -> WebSocketConnection:
    """Open a WebSocket :class:`Connection` to *url*.

    ``url`` is ``ws://`` or ``wss://``.  ``max_message_size=None`` (default)
    removes the per-frame size cap — needed for full-resolution display
    captures.
    """
    ws = await websockets.connect(
        url,
        ssl=ssl,
        additional_headers=additional_headers,
        max_size=max_message_size,
        # We use msgpack ourselves; turn off WebSocket-level compression.
        compression=None,
    )
    return WebSocketConnection(ws)


# ---------------------------------------------------------------------------
# Server
# ---------------------------------------------------------------------------


ConnectionHandler = Callable[[WebSocketConnection], Awaitable[None]]


class WebSocketServer:
    """A running WebSocket server.  Use via :func:`serve_websocket`."""

    def __init__(self, server: Server, host: str, port: int) -> None:
        self._server = server
        self.host = host
        self.port = port

    async def wait_closed(self) -> None:
        await self._server.wait_closed()

    def close(self) -> None:
        self._server.close()

    async def aclose(self) -> None:
        self._server.close()
        await self._server.wait_closed()

    async def __aenter__(self) -> "WebSocketServer":
        return self

    async def __aexit__(self, exc_type, exc, tb) -> None:
        await self.aclose()


async def serve_websocket(
    handler: ConnectionHandler,
    *,
    host: str = "0.0.0.0",
    port: int = 8423,
    ssl=None,
    max_message_size: Optional[int] = None,
) -> WebSocketServer:
    """Start a WebSocket server and invoke *handler* for each connection.

    *handler* receives a :class:`WebSocketConnection`.  When it returns (or
    raises), the connection is closed.

    Pass ``port=0`` to bind to an ephemeral port — read the actual port back
    from the returned :attr:`WebSocketServer.port`.
    """
    async def _handler(ws) -> None:
        conn = WebSocketConnection(ws)
        try:
            await handler(conn)
        except _WSClosed:
            pass
        finally:
            await conn.aclose()

    server = await websockets.serve(
        _handler,
        host=host,
        port=port,
        ssl=ssl,
        max_size=max_message_size,
        compression=None,
    )

    # Resolve the actual port when port=0 was requested.
    actual_port = port
    socks = getattr(server, "sockets", None) or []
    for s in socks:
        try:
            actual_port = s.getsockname()[1]
            break
        except Exception:  # pragma: no cover
            continue

    return WebSocketServer(server, host=host, port=actual_port)
