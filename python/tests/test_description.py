"""Tests for the controlled-machine description feature.

* Server-side: reading / writing / clearing `description.txt`; broadcast in
  the HELLO manifest.
* Controller-side: caching the broadcast description in trusted-peers;
  override wins over cached; CLI round-trip.
* MCP admin tools: `opendesk_peers` shows the truncated description,
  `opendesk_describe` returns the full text.
"""

from __future__ import annotations

import asyncio
import subprocess
import sys
from pathlib import Path

import pytest

from opendesk.integrations.mcp import MCPDispatcher
from opendesk.integrations.mcp_session import MCPSession
from opendesk.protocol.auth import Identity, TrustedPeers
from opendesk.registry import create_registry
from opendesk.remote.client import connect
from opendesk.remote.server import (
    OpendeskServer,
    clear_description,
    read_description,
    write_description,
)

from tests._fakes import FakeComputer


# ---------------------------------------------------------------------------
# Server-side description file
# ---------------------------------------------------------------------------


class TestDescriptionFile:
    def test_missing_file_returns_empty(self, tmp_path: Path):
        assert read_description(tmp_path) == ""

    def test_write_read_round_trip(self, tmp_path: Path):
        write_description(tmp_path, "ERP terminal — SAP installed.")
        assert read_description(tmp_path) == "ERP terminal — SAP installed."

    def test_clear(self, tmp_path: Path):
        write_description(tmp_path, "billing machine")
        assert clear_description(tmp_path) is True
        assert read_description(tmp_path) == ""
        # Clearing twice is a no-op.
        assert clear_description(tmp_path) is False

    def test_empty_string_stays_empty(self, tmp_path: Path):
        """Empty-string writes shouldn't create a file with content."""
        write_description(tmp_path, "")
        assert read_description(tmp_path) == ""


# ---------------------------------------------------------------------------
# End-to-end: server broadcasts → client caches
# ---------------------------------------------------------------------------


async def _server_with_desc(tmp_path: Path, desc: str) -> tuple[OpendeskServer, bytes]:
    fake = FakeComputer()
    server_home = tmp_path / "server"
    server_home.mkdir()
    write_description(server_home, desc)
    server_id = Identity.load_or_create(server_home)
    server_trusted = TrustedPeers(server_home)

    client_home = tmp_path / "client"
    client_home.mkdir()
    client_id = Identity.load_or_create(client_home)
    server_trusted.add(client_id.public_bytes, name="controller")
    TrustedPeers(client_home).add(server_id.public_bytes, name="host")

    server = OpendeskServer(
        fake, server_id, server_trusted,
        host="127.0.0.1", port=0,
        advertise_mdns=False, home=server_home,
    )
    await server.start()
    return server, server_id.public_bytes


class TestBroadcastAndCache:
    @pytest.mark.asyncio
    async def test_manifest_carries_description(self, tmp_path: Path):
        server, server_pub = await _server_with_desc(
            tmp_path, "billing machine — has Excel + Slack",
        )
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                auto_reconnect=False,
            )
            try:
                assert remote.capabilities().description == "billing machine — has Excel + Slack"
            finally:
                await remote.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_client_caches_description_into_trusted_peers(self, tmp_path: Path):
        server, server_pub = await _server_with_desc(
            tmp_path, "ERP terminal — SAP installed.",
        )
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                auto_reconnect=False,
            )
            await remote.aclose()
        finally:
            await server.aclose()

        # Fresh TrustedPeers instance reads from disk — proves cache landed.
        store = TrustedPeers(tmp_path / "client")
        peer = store.find_by_name("host")
        assert peer is not None
        assert peer.description == "ERP terminal — SAP installed."
        # No override yet, so effective == cached.
        assert peer.effective_description == "ERP terminal — SAP installed."

    @pytest.mark.asyncio
    async def test_description_picked_up_without_restart(self, tmp_path: Path):
        """`opendesk describe ...` edits show up on next session, no daemon restart."""
        server, server_pub = await _server_with_desc(tmp_path, "v1")
        try:
            r1 = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client", auto_reconnect=False,
            )
            assert r1.capabilities().description == "v1"
            await r1.aclose()

            # Operator changes the description while serve is running.
            write_description(tmp_path / "server", "v2 — updated")

            r2 = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client", auto_reconnect=False,
            )
            assert r2.capabilities().description == "v2 — updated"
            await r2.aclose()
        finally:
            await server.aclose()


# ---------------------------------------------------------------------------
# Controller-side override
# ---------------------------------------------------------------------------


class TestOverride:
    def test_override_wins_over_cached(self, tmp_path: Path):
        store = TrustedPeers(tmp_path)
        store.add(Identity.generate().public_bytes, name="mini")
        store.cache_description(store.find_by_name("mini").public_bytes, "from server")
        store.set_description_override("mini", "from controller")
        assert store.effective_description("mini") == "from controller"

    def test_clear_override_falls_back_to_cached(self, tmp_path: Path):
        store = TrustedPeers(tmp_path)
        store.add(Identity.generate().public_bytes, name="mini")
        store.cache_description(store.find_by_name("mini").public_bytes, "from server")
        store.set_description_override("mini", "from controller")
        assert store.clear_description_override("mini") is True
        assert store.effective_description("mini") == "from server"

    def test_set_override_for_unknown_peer_returns_false(self, tmp_path: Path):
        store = TrustedPeers(tmp_path)
        assert store.set_description_override("ghost", "x") is False


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


class TestDescribeCLI:
    def _opendesk(self, *args: str, home: Path) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, "-m", "opendesk.cli", *args],
            capture_output=True, text=True, timeout=10,
            env={"PATH": __import__("os").environ.get("PATH", "")},
        )

    def test_server_describe_round_trip(self, tmp_path: Path):
        # Show with none set.
        r = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "describe", "--home", str(tmp_path)],
            capture_output=True, text=True, timeout=10,
        )
        assert r.returncode == 0
        assert "(no description set)" in r.stdout

        # Set.
        r = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "describe",
             "billing machine — Excel + Slack", "--home", str(tmp_path)],
            capture_output=True, text=True, timeout=10,
        )
        assert r.returncode == 0
        assert read_description(tmp_path) == "billing machine — Excel + Slack"

        # Show.
        r = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "describe", "--home", str(tmp_path)],
            capture_output=True, text=True, timeout=10,
        )
        assert r.returncode == 0
        assert "billing machine" in r.stdout

        # Clear.
        r = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "describe", "--clear",
             "--home", str(tmp_path)],
            capture_output=True, text=True, timeout=10,
        )
        assert r.returncode == 0
        assert read_description(tmp_path) == ""

    def test_peers_describe_override(self, tmp_path: Path):
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")

        # Set override.
        r = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "peers", "describe", "mini",
             "Local label for the mini.", "--home", str(tmp_path)],
            capture_output=True, text=True, timeout=10,
        )
        assert r.returncode == 0
        assert TrustedPeers(tmp_path).effective_description("mini") == "Local label for the mini."

        # Show.
        r = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "peers", "describe", "mini",
             "--home", str(tmp_path)],
            capture_output=True, text=True, timeout=10,
        )
        assert r.returncode == 0
        assert "Local label" in r.stdout


# ---------------------------------------------------------------------------
# MCP admin tools
# ---------------------------------------------------------------------------


class TestAdminToolsDescription:
    @pytest.mark.asyncio
    async def test_opendesk_peers_shows_truncated_description(self, tmp_path: Path):
        store = TrustedPeers(tmp_path)
        store.add(Identity.generate().public_bytes, name="mini")
        store.cache_description(store.find_by_name("mini").public_bytes,
                                "mini in office — Slack, Notion, Excel")
        session = MCPSession(home=tmp_path, local=FakeComputer())
        dispatcher = MCPDispatcher(create_registry(), session)
        out = await dispatcher.call_tool("opendesk_peers", {})
        text = out[0].text  # type: ignore[union-attr]
        assert "mini in office" in text
        assert "opendesk_describe" in text  # the hint pointing to full text

    @pytest.mark.asyncio
    async def test_opendesk_describe_returns_full_text(self, tmp_path: Path):
        store = TrustedPeers(tmp_path)
        store.add(Identity.generate().public_bytes, name="mini")
        full = (
            "Mac mini in the office. Has Slack, Notion, Excel installed. "
            "Use this for billing-related tasks like invoice processing."
        )
        store.cache_description(store.find_by_name("mini").public_bytes, full)
        session = MCPSession(home=tmp_path, local=FakeComputer())
        dispatcher = MCPDispatcher(create_registry(), session)
        out = await dispatcher.call_tool("opendesk_describe", {"peer": "mini"})
        text = out[0].text  # type: ignore[union-attr]
        assert full in text
        assert "broadcast" in text.lower()

    @pytest.mark.asyncio
    async def test_opendesk_describe_with_override(self, tmp_path: Path):
        store = TrustedPeers(tmp_path)
        store.add(Identity.generate().public_bytes, name="mini")
        store.cache_description(store.find_by_name("mini").public_bytes, "from server")
        store.set_description_override("mini", "from controller (override)")
        session = MCPSession(home=tmp_path, local=FakeComputer())
        dispatcher = MCPDispatcher(create_registry(), session)
        out = await dispatcher.call_tool("opendesk_describe", {"peer": "mini"})
        text = out[0].text  # type: ignore[union-attr]
        assert "from controller (override)" in text
        assert "override" in text.lower()

    @pytest.mark.asyncio
    async def test_opendesk_describe_unknown_peer(self, tmp_path: Path):
        session = MCPSession(home=tmp_path, local=FakeComputer())
        dispatcher = MCPDispatcher(create_registry(), session)
        out = await dispatcher.call_tool("opendesk_describe", {"peer": "ghost"})
        text = out[0].text  # type: ignore[union-attr]
        assert "ERROR" in text
        assert "unknown peer" in text.lower()
