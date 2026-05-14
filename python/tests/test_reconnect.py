"""Reconnect resilience: a :class:`RemoteComputer` built with
``auto_reconnect=True`` survives transient WebSocket drops.

The test setup spins a real :class:`OpendeskServer`, connects through the
public :func:`opendesk.remote.client.connect` (which wires the connector
closure), and disrupts the connection server-side via
a transport-level connection close (no eviction PUSH).  That mimics the real failure: the
WebSocket goes away under the client, every in-flight call surfaces a
disconnect-style error.

Behaviour we lock in:

* Idempotent observation methods (``cursor_position``, ``capture``, ...)
  reconnect transparently and re-run.
* Side-effect methods (``pointer``, ``clipboard_write``) fail the in-flight
  call but the next call succeeds against the healed session.
* When the server is genuinely gone (port closed), the reconnect budget
  eventually exhausts and surfaces the original error.
"""

from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

from opendesk.computer import PointerAction, PointerEvent, Point
from opendesk.protocol.auth import Identity, TrustedPeers
from opendesk.remote.client import connect
from opendesk.remote.server import OpendeskServer

from tests._fakes import FakeComputer


async def _server_for(tmp_path: Path) -> tuple[OpendeskServer, FakeComputer, bytes]:
    fake = FakeComputer()
    server_id = Identity.load_or_create(tmp_path / "server")
    server_trusted = TrustedPeers(tmp_path / "server")
    client_id = Identity.load_or_create(tmp_path / "client")
    server_trusted.add(client_id.public_bytes, name="controller")
    TrustedPeers(tmp_path / "client").add(server_id.public_bytes, name="host")

    server = OpendeskServer(
        fake, server_id, server_trusted,
        host="127.0.0.1", port=0,
        advertise_mdns=False,
        home=tmp_path / "server",
    )
    await server.start()
    return server, fake, server_id.public_bytes


async def _wait_no_active_session(server: OpendeskServer, timeout: float = 2.0) -> None:
    """Wait until the server-side handler has finished cleaning up after
    a forced session close.  Polls because the cleanup is in a coroutine
    we don't directly await."""
    deadline = asyncio.get_event_loop().time() + timeout
    while asyncio.get_event_loop().time() < deadline:
        if not await server.sessions.list():
            return
        await asyncio.sleep(0.02)


async def _simulate_network_drop(server: OpendeskServer) -> int:
    """Force-close active sessions at the transport layer.

    Bypasses :meth:`SessionRegistry.kill` so no ``session.evicted`` PUSH is
    sent — the client sees a plain ``ConnectionClosed`` exactly as it would
    on a real Wi-Fi blip.  Used by the reconnect tests to verify the
    auto-recovery path (eviction is a *separate* path tested elsewhere).
    """
    dropped = 0
    for s in await server.sessions.list():
        peer = s.peer
        if peer is None:
            continue
        # Reach past peer.aclose() into the underlying encrypted connection
        # so the graceful close path doesn't get a chance to run.
        conn = getattr(peer, "_conn", None)
        if conn is not None:
            await conn.aclose()
            dropped += 1
    return dropped


# ---------------------------------------------------------------------------
# Idempotent path: transparent retry
# ---------------------------------------------------------------------------


class TestIdempotentReconnect:
    @pytest.mark.asyncio
    async def test_cursor_position_recovers_after_kill(self, tmp_path: Path):
        server, fake, server_pub = await _server_for(tmp_path)
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                reconnect_budget=5.0,
            )
            try:
                # Warm: the call works.
                pos = await remote.cursor_position()
                assert pos.x == 50

                # Simulate a transport-level network drop (no eviction PUSH).
                dropped = await _simulate_network_drop(server)
                assert dropped == 1
                await _wait_no_active_session(server)

                # Next idempotent call should transparently reconnect.
                pos2 = await remote.cursor_position()
                assert pos2.x == 50

                # Server now has a fresh session for our peer.
                sessions = await server.sessions.list()
                assert len(sessions) == 1
            finally:
                await remote.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_capture_recovers_after_kill(self, tmp_path: Path):
        """Binary payload survives reconnect end-to-end."""
        server, fake, server_pub = await _server_for(tmp_path)
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                reconnect_budget=5.0,
            )
            try:
                pix = await remote.capture()
                assert pix.data.startswith(b"\x89PNGfake")

                await _simulate_network_drop(server)
                await _wait_no_active_session(server)

                pix2 = await remote.capture()
                assert pix2.data.startswith(b"\x89PNGfake")
            finally:
                await remote.aclose()
        finally:
            await server.aclose()


# ---------------------------------------------------------------------------
# Side-effect path: don't replay; heal in background
# ---------------------------------------------------------------------------


class TestSideEffectReconnect:
    @pytest.mark.asyncio
    async def test_pointer_fails_but_next_call_succeeds(self, tmp_path: Path):
        """A pointer-down racing the disconnect surfaces an error
        (we won't replay an input event), but the connection heals so the
        following observation works without delay."""
        server, fake, server_pub = await _server_for(tmp_path)
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                reconnect_budget=5.0,
            )
            try:
                # Kill before sending the pointer event.
                await _simulate_network_drop(server)
                await _wait_no_active_session(server)

                # Non-idempotent call: failure is expected (we don't replay
                # an input event).  But the reconnect must run in the
                # background as part of the call, so the *next* call works.
                with pytest.raises(Exception):
                    await remote.pointer(PointerEvent(
                        action=PointerAction.MOVE, point=Point(x=10, y=20),
                    ))

                # Subsequent idempotent call should succeed on the freshly
                # reconnected session, with no further reconnect needed.
                pos = await remote.cursor_position()
                assert pos.x == 50
                assert len(await server.sessions.list()) == 1
            finally:
                await remote.aclose()
        finally:
            await server.aclose()


# ---------------------------------------------------------------------------
# Budget exhaustion: server gone for good
# ---------------------------------------------------------------------------


class TestReconnectBudget:
    @pytest.mark.asyncio
    async def test_call_fails_when_server_is_gone(self, tmp_path: Path):
        server, fake, server_pub = await _server_for(tmp_path)
        remote = await connect(
            f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
            home=tmp_path / "client",
            reconnect_budget=1.0,  # short for the test
        )
        try:
            pos = await remote.cursor_position()
            assert pos.x == 50

            # Take the whole server down.  Reconnect attempts will all fail
            # (TCP connection refused) and the budget will eventually expire.
            await server.aclose()

            with pytest.raises(Exception):
                await remote.cursor_position()
        finally:
            await remote.aclose()


# ---------------------------------------------------------------------------
# Opt-out: auto_reconnect=False keeps the v0 behaviour
# ---------------------------------------------------------------------------


class TestAutoReconnectOptOut:
    @pytest.mark.asyncio
    async def test_kill_fails_subsequent_calls_when_opted_out(self, tmp_path: Path):
        server, fake, server_pub = await _server_for(tmp_path)
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                auto_reconnect=False,
            )
            try:
                await remote.cursor_position()
                await _simulate_network_drop(server)
                await _wait_no_active_session(server)
                with pytest.raises(Exception):
                    await remote.cursor_position()
            finally:
                await remote.aclose()
        finally:
            await server.aclose()
