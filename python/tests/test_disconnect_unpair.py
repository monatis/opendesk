"""Tests for the redesigned single-controller CLI:

* ``opendesk sessions`` — list-only.
* ``opendesk disconnect`` — kicks the active session.
* ``opendesk unpair <name>`` — revokes trust *and* disconnects if active.
* ``opendesk peers remove <name>`` — same as ``unpair`` (kept as alias).
"""

from __future__ import annotations

import asyncio
import subprocess
import sys
from pathlib import Path

import pytest

from opendesk.protocol.auth import Identity, TrustedPeers
from opendesk.remote.admin import AdminClient
from opendesk.remote.client import connect, pair_with
from opendesk.remote.server import OpendeskServer, SessionInfo, ServerMode

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


# ---------------------------------------------------------------------------
# disconnect
# ---------------------------------------------------------------------------


class TestDisconnect:
    @pytest.mark.asyncio
    async def test_disconnect_kicks_active_session(self, tmp_path: Path):
        server, _, server_pub = await _server(tmp_path)
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                auto_reconnect=False,
            )
            try:
                await asyncio.sleep(0.05)
                assert len(await server.sessions.list()) == 1

                client = await AdminClient.connect(home=tmp_path / "server")
                try:
                    killed = await client.kill_all()
                    assert killed == 1
                finally:
                    await client.aclose()

                await asyncio.sleep(0.05)
                assert await server.sessions.list() == []
            finally:
                with __import__("contextlib").suppress(Exception):
                    await remote.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_disconnect_with_no_active_session(self, tmp_path: Path):
        server, _, _ = await _server(tmp_path)
        try:
            client = await AdminClient.connect(home=tmp_path / "server")
            try:
                killed = await client.kill_all()
                assert killed == 0
            finally:
                await client.aclose()
        finally:
            await server.aclose()


# ---------------------------------------------------------------------------
# unpair
# ---------------------------------------------------------------------------


class TestUnpair:
    @pytest.mark.asyncio
    async def test_unpair_removes_from_trusted_and_kicks(self, tmp_path: Path):
        """End-to-end: pair with a peer (it then has an active session),
        unpair them, the session is dropped and they can't reconnect."""
        server, _, server_pub = await _server(tmp_path)
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                auto_reconnect=False,
            )
            try:
                await asyncio.sleep(0.05)
                # Simulate `opendesk unpair laptop`:
                from opendesk.cli import _disconnect_if_active

                store = TrustedPeers(tmp_path / "server")
                assert store.remove("laptop") is True
                kicked = await _disconnect_if_active(tmp_path / "server", "laptop")
                assert kicked is True

                # The trusted-peers file no longer mentions the peer.
                assert not store.contains(
                    Identity.load_or_create(tmp_path / "client").public_bytes
                )
                # The server's session was cleaned up.
                await asyncio.sleep(0.05)
                assert await server.sessions.list() == []
            finally:
                with __import__("contextlib").suppress(Exception):
                    await remote.aclose()
        finally:
            await server.aclose()


# ---------------------------------------------------------------------------
# Smoke tests — CLI subcommands wire up correctly
# ---------------------------------------------------------------------------


class TestCLIWired:
    def test_disconnect_subcommand_exists(self):
        r = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "disconnect", "--help"],
            capture_output=True, text=True, timeout=10,
        )
        assert r.returncode == 0
        assert "disconnect" in r.stdout.lower()

    def test_unpair_subcommand_exists(self):
        r = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "unpair", "--help"],
            capture_output=True, text=True, timeout=10,
        )
        assert r.returncode == 0
        assert "name" in r.stdout.lower()

    def test_sessions_kill_subcommand_removed(self):
        r = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "sessions", "kill", "x"],
            capture_output=True, text=True, timeout=10,
        )
        # `sessions` no longer takes a `kill` subcommand.
        assert r.returncode != 0

    def test_top_level_help_lists_disconnect_unpair(self):
        r = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "--help"],
            capture_output=True, text=True, timeout=10,
        )
        assert r.returncode == 0
        assert "disconnect" in r.stdout
        assert "unpair" in r.stdout
