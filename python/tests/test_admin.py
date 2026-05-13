"""Tests for the local admin IPC used by ``opendesk sessions``.

End-to-end via :class:`AdminServer` + :class:`AdminClient` over a real Unix
domain socket (or localhost TCP on Windows) bound to a tmpdir.
"""

from __future__ import annotations

import asyncio
import sys
from pathlib import Path

import pytest

from opendesk.protocol.auth import Identity, TrustedPeers
from opendesk.remote.admin import AdminClient, AdminError, AdminServer
from opendesk.remote.server import OpendeskServer

from tests._fakes import FakeComputer


async def _build_server(tmp_path: Path) -> OpendeskServer:
    fake = FakeComputer()
    identity = Identity.load_or_create(tmp_path)
    trusted = TrustedPeers(tmp_path)
    server = OpendeskServer(
        fake, identity, trusted,
        host="127.0.0.1", port=0,
        advertise_mdns=False,
        home=tmp_path,
    )
    await server.start()
    return server


class TestAdminLifecycle:
    @pytest.mark.asyncio
    async def test_admin_socket_is_created_on_start(self, tmp_path: Path):
        server = await _build_server(tmp_path)
        try:
            if sys.platform == "win32":
                assert (tmp_path / "admin.port").exists()
            else:
                sock = tmp_path / "admin.sock"
                assert sock.exists()
                # 0600 permissions
                assert sock.stat().st_mode & 0o077 == 0
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_admin_socket_removed_on_close(self, tmp_path: Path):
        server = await _build_server(tmp_path)
        await server.aclose()
        if sys.platform == "win32":
            assert not (tmp_path / "admin.port").exists()
        else:
            assert not (tmp_path / "admin.sock").exists()


class TestAdminClient:
    @pytest.mark.asyncio
    async def test_connect_fails_when_no_server(self, tmp_path: Path):
        with pytest.raises(AdminError):
            await AdminClient.connect(home=tmp_path)

    @pytest.mark.asyncio
    async def test_list_with_no_sessions(self, tmp_path: Path):
        server = await _build_server(tmp_path)
        try:
            client = await AdminClient.connect(home=tmp_path)
            try:
                sessions = await client.list_sessions()
                assert sessions == []
            finally:
                await client.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_list_after_a_session_starts(self, tmp_path: Path):
        """Spin up the server, simulate one connected peer in the registry,
        verify the admin list reflects it."""
        from opendesk.remote.server import SessionInfo, ServerMode

        server = await _build_server(tmp_path)
        try:
            # Inject a synthetic session into the registry.
            await server.sessions.add(SessionInfo(
                id="abcd",
                peer_public=b"\x00" * 32,
                peer_name="laptop",
                remote_addr="192.168.1.10:54321",
                mode=ServerMode.SERVE,
            ))

            client = await AdminClient.connect(home=tmp_path)
            try:
                sessions = await client.list_sessions()
                assert len(sessions) == 1
                s = sessions[0]
                assert s["id"] == "abcd"
                assert s["peer_name"] == "laptop"
                assert s["remote_addr"] == "192.168.1.10:54321"
                assert s["mode"] == "serve"
                assert isinstance(s["age_seconds"], (int, float))
            finally:
                await client.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_kill_unknown_session_returns_false(self, tmp_path: Path):
        server = await _build_server(tmp_path)
        try:
            client = await AdminClient.connect(home=tmp_path)
            try:
                ok = await client.kill("nope")
                assert ok is False
            finally:
                await client.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_kill_all_when_empty_returns_zero(self, tmp_path: Path):
        server = await _build_server(tmp_path)
        try:
            client = await AdminClient.connect(home=tmp_path)
            try:
                killed = await client.kill_all()
                assert killed == 0
            finally:
                await client.aclose()
        finally:
            await server.aclose()


class TestCLISessionsWired:
    def test_sessions_subcommand_exists(self):
        import subprocess
        result = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "sessions", "--help"],
            capture_output=True, text=True, timeout=10,
        )
        assert result.returncode == 0
        assert "sessions" in result.stdout

    def test_top_level_help_lists_sessions(self):
        import subprocess
        result = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "--help"],
            capture_output=True, text=True, timeout=10,
        )
        assert result.returncode == 0
        assert "sessions" in result.stdout
