"""Tests for cooperative eviction.

The server can ask a controller to disconnect *without* breaking trust by
sending a ``session.evicted`` PUSH frame before closing the connection.  A
cooperative client (the in-tree :class:`RemoteComputer`) flips its evicted
flag, refuses to auto-reconnect, and raises :class:`SessionEvicted` on
subsequent calls.

These tests verify both halves of the cooperation:

* server-side ``SessionRegistry.kill`` sends the PUSH and closes;
* client-side ``RemoteComputer`` honours it.
"""

from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

from opendesk.computer import SessionEvicted
from opendesk.protocol.auth import Identity, TrustedPeers
from opendesk.remote.client import connect
from opendesk.remote.server import OpendeskServer

from tests._fakes import FakeComputer


async def _server(tmp_path: Path) -> tuple[OpendeskServer, FakeComputer, bytes]:
    fake = FakeComputer()
    server_id = Identity.load_or_create(tmp_path / "server")
    server_trusted = TrustedPeers(tmp_path / "server")
    client_id = Identity.load_or_create(tmp_path / "client")
    server_trusted.add(client_id.public_bytes, name="laptop")
    TrustedPeers(tmp_path / "client").add(server_id.public_bytes, name="host")
    server = OpendeskServer(
        fake, server_id, server_trusted,
        host="127.0.0.1", port=0,
        advertise_mdns=False,
        home=tmp_path / "server",
    )
    await server.start()
    return server, fake, server_id.public_bytes


async def _wait_no_active(server: OpendeskServer, timeout: float = 2.0) -> None:
    deadline = asyncio.get_event_loop().time() + timeout
    while asyncio.get_event_loop().time() < deadline:
        if not await server.sessions.list():
            return
        await asyncio.sleep(0.02)


# ---------------------------------------------------------------------------
# Cooperative eviction
# ---------------------------------------------------------------------------


class TestEviction:
    @pytest.mark.asyncio
    async def test_kill_marks_client_as_evicted(self, tmp_path: Path):
        server, _, server_pub = await _server(tmp_path)
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                reconnect_budget=5.0,
            )
            try:
                await remote.cursor_position()
                assert remote.evicted is False

                # Server-initiated eviction (the admin-IPC path uses this).
                killed = await server.sessions.kill_all()
                assert killed == 1
                # Give the PUSH frame a tick to land on the client.
                await asyncio.sleep(0.1)
                assert remote.evicted is True
                assert remote.eviction_reason == "admin_disconnect"
            finally:
                with __import__("contextlib").suppress(Exception):
                    await remote.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_evicted_call_raises_session_evicted(self, tmp_path: Path):
        server, _, server_pub = await _server(tmp_path)
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                reconnect_budget=5.0,
            )
            try:
                await remote.cursor_position()
                await server.sessions.kill_all()
                await asyncio.sleep(0.1)

                with pytest.raises(SessionEvicted) as exc_info:
                    await remote.cursor_position()
                assert exc_info.value.reason == "admin_disconnect"
            finally:
                with __import__("contextlib").suppress(Exception):
                    await remote.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_eviction_suppresses_reconnect(self, tmp_path: Path):
        """The whole point: even with auto_reconnect, the client respects the
        server's request and does NOT come back."""
        server, _, server_pub = await _server(tmp_path)
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                reconnect_budget=5.0,
            )
            try:
                await remote.cursor_position()
                await server.sessions.kill_all()
                await asyncio.sleep(0.1)

                # The next call must raise (SessionEvicted) instead of
                # silently reopening a session.
                with pytest.raises(SessionEvicted):
                    await remote.cursor_position()

                # Give the supposed reconnect a chance — there should be
                # none.
                await asyncio.sleep(0.5)
                assert await server.sessions.list() == []
            finally:
                with __import__("contextlib").suppress(Exception):
                    await remote.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_eviction_does_not_revoke_trust(self, tmp_path: Path):
        """After eviction, trust is intact — a fresh `connect()` (i.e. new
        RemoteComputer / new connector closure) succeeds."""
        server, _, server_pub = await _server(tmp_path)
        try:
            evicted = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                reconnect_budget=5.0,
            )
            await evicted.cursor_position()
            await server.sessions.kill_all()
            await asyncio.sleep(0.1)
            assert evicted.evicted is True
            with __import__("contextlib").suppress(Exception):
                await evicted.aclose()

            # Brand new RemoteComputer — should still authenticate.
            fresh = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
            )
            try:
                pos = await fresh.cursor_position()
                assert pos.x == 50
            finally:
                await fresh.aclose()
        finally:
            await server.aclose()
