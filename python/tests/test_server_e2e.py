"""End-to-end tests for the full opendesk remote stack.

These exercise everything together: a real :class:`OpendeskServer` listening
on ``127.0.0.1`` with a real :class:`Identity` and :class:`TrustedPeers`
store; a real client that pairs and then reconnects; and a
:class:`RemoteComputer` driving a :class:`FakeComputer` through the
authenticated, encrypted tunnel.

If these pass, the LAN flow works modulo mDNS — which is exercised by
:mod:`tests.test_discovery`.
"""

from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

from opendesk.computer import (
    Capability,
    PointerAction,
    PointerEvent,
    Point,
    Rect,
)
from opendesk.protocol.auth import (
    AuthFailure,
    Identity,
    TrustedPeers,
)
from opendesk.remote.client import connect, pair_with
from opendesk.remote.server import OpendeskServer

from tests._fakes import FakeComputer


async def _start_server(tmp_path: Path) -> tuple[OpendeskServer, FakeComputer]:
    fake = FakeComputer()
    identity = Identity.load_or_create(tmp_path)
    trusted = TrustedPeers(tmp_path)
    server = OpendeskServer(
        fake, identity, trusted,
        host="127.0.0.1", port=0,
        advertise_mdns=False,
        home=tmp_path,  # critical: without this, audit / admin-socket land
                        # in the real ~/.opendesk and pollute the user's data
    )
    await server.start()
    return server, fake


# ---------------------------------------------------------------------------
# Pairing
# ---------------------------------------------------------------------------


class TestPairingE2E:
    @pytest.mark.asyncio
    async def test_full_pair_lifecycle(self, tmp_path: Path):
        server, fake = await _start_server(tmp_path / "server")
        try:
            code = "424242"

            async def pair_side():
                return await server.enable_pairing(code, timeout=10)

            async def client_side():
                # Client gets its own home for its identity / trusted-peers.
                home = tmp_path / "client"
                remote, server_pub = await pair_with(
                    host="127.0.0.1", port=server.port, code=code,
                    home=home, name="mac-mini",
                )
                return remote, server_pub, home

            pair_task = asyncio.create_task(pair_side())
            client_task = asyncio.create_task(client_side())
            server_new_pub, (remote, returned_server_pub, client_home) = await asyncio.gather(
                pair_task, client_task,
            )

            # Server should now trust the client's key.
            assert server_new_pub is not None
            srv_trusted = TrustedPeers(tmp_path / "server")
            assert srv_trusted.contains(server_new_pub)

            # Client should have stored the server's key.
            cli_trusted = TrustedPeers(client_home)
            assert cli_trusted.contains(returned_server_pub)
            assert cli_trusted.find_by_name("mac-mini") is not None

            # And the resulting RemoteComputer works end-to-end.
            caps = remote.capabilities()
            assert caps.has(Capability.DISPLAY_CAPTURE)
            pos = await remote.cursor_position()
            assert pos.x == 50

            await remote.aclose()
        finally:
            await server.aclose()


# ---------------------------------------------------------------------------
# Reconnect
# ---------------------------------------------------------------------------


class TestReconnectE2E:
    @pytest.mark.asyncio
    async def test_pair_then_reconnect_full_stack(self, tmp_path: Path):
        # Step 1: pair.
        server, fake = await _start_server(tmp_path / "server")
        code = "112233"
        pair_task = asyncio.create_task(server.enable_pairing(code, timeout=10))
        remote, server_pub = await pair_with(
            host="127.0.0.1", port=server.port, code=code,
            home=tmp_path / "client", name="mini",
        )
        await pair_task
        await remote.aclose()

        # Step 2: reconnect using the stored keys + name.
        try:
            remote2 = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
            )
            try:
                # Drive a real Computer call through the authenticated tunnel.
                await remote2.pointer(PointerEvent(
                    action=PointerAction.MOVE, point=Point(x=42, y=42),
                ))
                pixmap = await remote2.capture(region=Rect(x=0, y=0, width=10, height=10))
                assert isinstance(pixmap.data, bytes)
                assert pixmap.data.startswith(b"\x89PNGfake")
                assert any(c[0] == "pointer" for c in fake.calls)
            finally:
                await remote2.aclose()
        finally:
            await server.aclose()


# ---------------------------------------------------------------------------
# Auth failure modes
# ---------------------------------------------------------------------------


class TestRejection:
    @pytest.mark.asyncio
    async def test_untrusted_client_rejected_by_server(self, tmp_path: Path):
        """A client with a fresh identity tries to connect to a server with
        an empty trusted-peers list.  The server should refuse and the client
        should see an AuthFailure."""
        server, _ = await _start_server(tmp_path / "server")
        try:
            # Don't pair.  Just try to connect with an arbitrary expected pubkey.
            with pytest.raises(Exception):
                await connect(
                    f"ws://127.0.0.1:{server.port}#{server.public_key.hex()}",
                    home=tmp_path / "client",
                )
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_wrong_pairing_code_rejected(self, tmp_path: Path):
        server, _ = await _start_server(tmp_path / "server")
        try:
            pair_task = asyncio.create_task(server.enable_pairing("111111", timeout=10))

            with pytest.raises(AuthFailure) as exc_info:
                await pair_with(
                    host="127.0.0.1", port=server.port, code="999999",
                    home=tmp_path / "client",
                )
            assert exc_info.value.reason == "wrong_code"

            # The pair_task is still running because pairing didn't succeed.
            # Cancel it so the test cleans up.
            pair_task.cancel()
            try:
                await pair_task
            except (asyncio.CancelledError, BaseException):
                pass
        finally:
            await server.aclose()


# ---------------------------------------------------------------------------
# Sessions registry
# ---------------------------------------------------------------------------


class TestSessions:
    @pytest.mark.asyncio
    async def test_active_session_appears_in_registry(self, tmp_path: Path):
        server, _ = await _start_server(tmp_path / "server")
        try:
            code = "777777"
            pair_task = asyncio.create_task(server.enable_pairing(code, timeout=10))
            remote, server_pub = await pair_with(
                host="127.0.0.1", port=server.port, code=code,
                home=tmp_path / "client", name="laptop",
            )
            await pair_task

            try:
                # Allow time for the server-side session to be registered.
                await asyncio.sleep(0.05)
                sessions = await server.sessions.list()
                assert len(sessions) == 1
                s = sessions[0]
                # The session records the *client's* static public key.
                client_identity = Identity.load_or_create(tmp_path / "client")
                assert s.peer_public == client_identity.public_bytes
            finally:
                await remote.aclose()
        finally:
            await server.aclose()
