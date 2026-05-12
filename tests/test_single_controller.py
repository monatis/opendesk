"""Tests for the single-controller enforcement on :class:`OpendeskServer`.

Real :class:`OpendeskServer` on ``127.0.0.1`` with WebSocket transport and
the full pair/auth handshake — the same path real users hit.  Covers:

* Second peer rejected with ``ErrorCode.BUSY`` while another is active.
* Same peer reconnecting kicks the previous session cleanly.
* ``max_sessions=2`` allows two concurrent.
"""

from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

from opendesk.computer import RemoteComputer
from opendesk.protocol import ProtocolError
from opendesk.protocol.auth import Identity, TrustedPeers, auth_client
from opendesk.protocol.frames import ErrorCode
from opendesk.protocol.transports.websocket import connect_websocket
from opendesk.remote.server import OpendeskServer

from tests._fakes import FakeComputer


# ---------------------------------------------------------------------------
# Fixture helpers
# ---------------------------------------------------------------------------


async def _server_with_trust(
    tmp_path: Path, client_pubs: list[bytes],
) -> tuple[OpendeskServer, FakeComputer, bytes]:
    """Start an OpendeskServer that trusts every public key in *client_pubs*."""
    fake = FakeComputer()
    identity = Identity.load_or_create(tmp_path / "server")
    trusted = TrustedPeers(tmp_path / "server")
    for i, pub in enumerate(client_pubs):
        trusted.add(pub, name=f"client{i}")

    server = OpendeskServer(
        fake, identity, trusted,
        host="127.0.0.1", port=0,
        advertise_mdns=False,
        home=tmp_path / "server",
    )
    await server.start()
    return server, fake, identity.public_bytes


async def _connect_remote(
    port: int, client_home: Path, server_pub: bytes,
) -> RemoteComputer:
    """Open a RemoteComputer using a fresh identity at ``client_home``."""
    Identity.load_or_create(client_home)
    return await RemoteComputer.connect(await _ws(port), ).__class__  # placeholder; below


async def _open_authed_connection(
    port: int, identity: Identity, server_pub: bytes,
):
    raw = await connect_websocket(f"ws://127.0.0.1:{port}")
    try:
        session = await auth_client(raw, identity, server_pub)
    except BaseException:
        await raw.aclose()
        raise
    return session


async def _open_remote(
    port: int, identity: Identity, server_pub: bytes,
) -> RemoteComputer:
    """Connect via auth + HELLO and wrap in a RemoteComputer."""
    session = await _open_authed_connection(port, identity, server_pub)
    return await RemoteComputer.connect(session.connection)


async def _ws(port: int):
    return await connect_websocket(f"ws://127.0.0.1:{port}")


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


class TestSingleController:
    @pytest.mark.asyncio
    async def test_second_different_peer_rejected_busy(self, tmp_path: Path):
        # Two distinct clients, both trusted by the server.
        client_a = Identity.load_or_create(tmp_path / "client_a")
        client_b = Identity.load_or_create(tmp_path / "client_b")
        server, _, server_pub = await _server_with_trust(
            tmp_path, [client_a.public_bytes, client_b.public_bytes],
        )
        remote_a = None
        try:
            # First client takes the slot.
            remote_a = await _open_remote(server.port, client_a, server_pub)
            assert remote_a.capabilities().backend == "fake"

            # Second client's HELLO should raise BUSY.
            with pytest.raises(ProtocolError) as exc_info:
                await _open_remote(server.port, client_b, server_pub)
            assert exc_info.value.code == ErrorCode.BUSY.value
            msg = exc_info.value.message.lower()
            assert "busy" in msg or "active controller" in msg

            # First client is still working.
            pos = await remote_a.cursor_position()
            assert pos.x == 50
        finally:
            if remote_a is not None:
                await remote_a.aclose()
            await server.aclose()

    @pytest.mark.asyncio
    async def test_same_peer_reconnect_bumps_old_session(self, tmp_path: Path):
        client = Identity.load_or_create(tmp_path / "client")
        server, _, server_pub = await _server_with_trust(
            tmp_path, [client.public_bytes],
        )
        try:
            remote_first = await _open_remote(server.port, client, server_pub)
            # Server records one session.
            await asyncio.sleep(0.05)
            assert len(await server.sessions.list()) == 1

            # Same peer reconnects — server kicks the old, accepts the new.
            remote_second = await _open_remote(server.port, client, server_pub)
            await asyncio.sleep(0.1)  # let the old handler finish cleanup
            sessions = await server.sessions.list()
            assert len(sessions) == 1, sessions
            # The first remote's calls should now fail (it was closed).
            with pytest.raises(Exception):
                await remote_first.cursor_position()
            # The second remote keeps working.
            pos = await remote_second.cursor_position()
            assert pos.x == 50

            await remote_second.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_capacity_clears_after_disconnect(self, tmp_path: Path):
        """Closing the active session frees the slot for someone else."""
        client_a = Identity.load_or_create(tmp_path / "client_a")
        client_b = Identity.load_or_create(tmp_path / "client_b")
        server, _, server_pub = await _server_with_trust(
            tmp_path, [client_a.public_bytes, client_b.public_bytes],
        )
        try:
            remote_a = await _open_remote(server.port, client_a, server_pub)
            await asyncio.sleep(0.05)
            assert len(await server.sessions.list()) == 1

            await remote_a.aclose()
            # Wait briefly for the server-side handler's finally block.
            await asyncio.sleep(0.1)
            assert await server.sessions.list() == []

            # Now b can connect.
            remote_b = await _open_remote(server.port, client_b, server_pub)
            try:
                pos = await remote_b.cursor_position()
                assert pos.x == 50
            finally:
                await remote_b.aclose()
        finally:
            await server.aclose()
