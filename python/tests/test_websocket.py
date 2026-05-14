"""End-to-end tests over a real WebSocket connection.

These mirror the loopback tests but use :func:`serve_websocket` +
:func:`connect_websocket` so the full stack — TCP, the WebSocket framing
library, our msgpack codec, the Peer correlator, the ComputerDispatcher,
and RemoteComputer — runs against actual sockets on ``127.0.0.1:0``.

If these pass, the only thing still in the way of remote control across a
LAN is auth + discovery.
"""

from __future__ import annotations

import asyncio
from typing import AsyncIterator

import pytest

from opendesk.computer import (
    Capability,
    ComputerDispatcher,
    PointerAction,
    PointerEvent,
    Point,
    Rect,
    RemoteComputer,
)
from opendesk.protocol import Peer
from opendesk.protocol.transports.websocket import (
    WebSocketConnection,
    WebSocketServer,
    connect_websocket,
    serve_websocket,
)

from tests._fakes import FakeComputer


# ---------------------------------------------------------------------------
# Test fixture helpers
# ---------------------------------------------------------------------------


async def _start_server() -> tuple[WebSocketServer, FakeComputer, list[Peer]]:
    """Start a WebSocket server backed by a FakeComputer.

    Returns the running server, the fake (so tests can inspect calls), and
    a list that collects server-side Peers (so tests can wait on cleanup).
    """
    fake = FakeComputer()
    peers: list[Peer] = []

    async def handle(conn: WebSocketConnection) -> None:
        peer = Peer(conn, role="server", dispatcher=ComputerDispatcher(fake))
        peers.append(peer)
        await peer.hello(fake.capabilities().model_dump())
        peer.start()
        await peer.wait_closed()

    server = await serve_websocket(handle, host="127.0.0.1", port=0)
    return server, fake, peers


async def _connect(server: WebSocketServer) -> RemoteComputer:
    conn = await connect_websocket(f"ws://127.0.0.1:{server.port}")
    return await RemoteComputer.connect(conn)


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


class TestWebSocketRoundTrip:
    @pytest.mark.asyncio
    async def test_hello_handshake_over_socket(self):
        server, fake, peers = await _start_server()
        try:
            remote = await _connect(server)
            try:
                manifest = remote.capabilities()
                assert manifest.has(Capability.DISPLAY_CAPTURE)
                assert manifest.backend == "fake"
            finally:
                await remote.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_cursor_position_round_trip(self):
        server, fake, peers = await _start_server()
        try:
            remote = await _connect(server)
            try:
                pos = await remote.cursor_position()
                assert pos.x == 50 and pos.y == 60
            finally:
                await remote.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_binary_capture_payload_intact_over_socket(self):
        """The fully-end-to-end binary path: bytes leave the server as
        msgpack-encoded WebSocket binary frames and arrive at the client as
        bytes, no inflation, no base64.
        """
        server, fake, peers = await _start_server()
        try:
            remote = await _connect(server)
            try:
                pixmap = await remote.capture(region=Rect(x=0, y=0, width=10, height=10))
                assert isinstance(pixmap.data, bytes)
                assert pixmap.data.startswith(b"\x89PNGfake")
            finally:
                await remote.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_pointer_event_traverses_socket(self):
        server, fake, peers = await _start_server()
        try:
            remote = await _connect(server)
            try:
                await remote.pointer(PointerEvent(
                    action=PointerAction.MOVE, point=Point(x=10, y=20),
                ))
                # Allow the server task one event-loop tick to record the call.
                await asyncio.sleep(0.05)
                assert any(c[0] == "pointer" for c in fake.calls)
            finally:
                await remote.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_display_stream_over_socket(self):
        server, fake, peers = await _start_server()
        try:
            remote = await _connect(server)
            try:
                frames = []
                async for frame in remote.subscribe_display(fps=30):
                    frames.append(frame)
                assert len(frames) == 3
                assert frames[0].pixmap.data == b"frame\x00"
                assert frames[2].pixmap.data == b"frame\x02"
            finally:
                await remote.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_concurrent_connections(self):
        """Two clients against the same server, same FakeComputer underneath."""
        server, fake, peers = await _start_server()
        try:
            remote_a = await _connect(server)
            remote_b = await _connect(server)
            try:
                pos_a = await remote_a.cursor_position()
                pos_b = await remote_b.cursor_position()
                assert pos_a.x == 50 and pos_b.x == 50
                # Two peers should be tracked on the server side.
                await asyncio.sleep(0.05)
                assert len(peers) == 2
            finally:
                await remote_a.aclose()
                await remote_b.aclose()
        finally:
            await server.aclose()
