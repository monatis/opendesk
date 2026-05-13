"""Tests for the MCP integration.

Exercises the library-neutral :class:`MCPDispatcher` directly so the tests
don't depend on running an MCP transport.  The dispatcher is what
:func:`create_mcp_server` wraps with ``mcp.types.*`` conversion, so testing
it covers the full agent-facing surface.
"""

from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

from opendesk.computer import LocalComputer
from opendesk.integrations.mcp import (
    MCPDispatcher,
    PEER_AWARE_TOOLS,
    TextResult,
    ImageResult,
)
from opendesk.integrations.mcp_session import LOCAL, MCPSession, MCPSessionError
from opendesk.protocol.auth import Identity, TrustedPeers
from opendesk.registry import create_registry

from tests._fakes import FakeComputer


# ---------------------------------------------------------------------------
# MCPSession unit tests (no MCP package involved)
# ---------------------------------------------------------------------------


class TestMCPSession:
    @pytest.mark.asyncio
    async def test_default_resolves_to_local_with_no_peers(self, tmp_path: Path):
        local = FakeComputer()
        session = MCPSession(home=tmp_path, local=local)
        computer, name = await session.resolve()
        assert computer is local
        assert name == LOCAL

    @pytest.mark.asyncio
    async def test_explicit_local_resolves_to_local(self, tmp_path: Path):
        local = FakeComputer()
        session = MCPSession(home=tmp_path, local=local)
        computer, name = await session.resolve("local")
        assert computer is local
        assert name == LOCAL

    def test_use_peer_validates_against_trusted(self, tmp_path: Path):
        session = MCPSession(home=tmp_path)
        with pytest.raises(MCPSessionError):
            session.use_peer("nonexistent")

    def test_use_peer_accepts_trusted(self, tmp_path: Path):
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        session = MCPSession(home=tmp_path)
        session.use_peer("mini")
        assert session.current_peer == "mini"

    def test_use_local_reverts(self, tmp_path: Path):
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        session = MCPSession(home=tmp_path)
        session.use_peer("mini")
        session.use_peer("local")
        assert session.current_peer is None


class TestImplicitDefault:
    """Single paired peer = implicit default — the ergonomic single-machine case."""

    @pytest.mark.asyncio
    async def test_one_trusted_peer_becomes_implicit_default(self, tmp_path: Path):
        local = FakeComputer()
        remote = FakeComputer()
        session = _session_with_remote(tmp_path, local, remote, name="mini")
        # No explicit use_peer() call.
        assert session.current_peer is None
        name, source = session.effective_peer()
        assert name == "mini" and source == "implicit"
        # resolve() without an argument picks the implicit one.
        computer, resolved = await session.resolve()
        assert resolved == "mini"
        assert computer is remote

    @pytest.mark.asyncio
    async def test_zero_peers_resolves_to_local(self, tmp_path: Path):
        local = FakeComputer()
        session = MCPSession(home=tmp_path, local=local)
        name, source = session.effective_peer()
        assert name is None and source == "local"
        computer, resolved = await session.resolve()
        assert computer is local and resolved == LOCAL

    @pytest.mark.asyncio
    async def test_two_trusted_peers_no_default_raises(self, tmp_path: Path):
        """Multiple peers + no explicit default = ambiguous; must raise."""
        local = FakeComputer()
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="desktop")
        session = MCPSession(home=tmp_path, local=local)
        name, source = session.effective_peer()
        assert name is None and source == "ambiguous"
        with pytest.raises(MCPSessionError) as exc_info:
            await session.resolve()
        assert "mini" in str(exc_info.value)
        assert "desktop" in str(exc_info.value)

    @pytest.mark.asyncio
    async def test_two_peers_explicit_local_still_works(self, tmp_path: Path):
        """The ambiguous case is only triggered by an *omitted* peer arg."""
        local = FakeComputer()
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="desktop")
        session = MCPSession(home=tmp_path, local=local)
        computer, resolved = await session.resolve("local")
        assert computer is local and resolved == LOCAL

    @pytest.mark.asyncio
    async def test_two_peers_explicit_peer_arg_works(self, tmp_path: Path):
        """Per-call `peer:` resolves a specific target in the ambiguous setup."""
        local = FakeComputer()
        remote = FakeComputer()
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="desktop")
        session = _session_with_remote(tmp_path, local, remote, name="mini")
        # Now there are two trusted peers (mini + desktop) but mini is cached.
        computer, resolved = await session.resolve("mini")
        assert computer is remote and resolved == "mini"

    @pytest.mark.asyncio
    async def test_ambiguous_resolution_through_tool_call(self, tmp_path: Path):
        """Computer-tool dispatch surfaces the ambiguity as a clear error."""
        local = FakeComputer()
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="desktop")
        session = MCPSession(home=tmp_path, local=local)
        dispatcher = MCPDispatcher(create_registry(), session)
        result = await dispatcher.call_tool("clipboard", {"action": "read"})
        text = result[0].text  # type: ignore[union-attr]
        assert "ERROR" in text
        assert "ambiguous" in text.lower() or "multiple peers" in text.lower()

    @pytest.mark.asyncio
    async def test_admin_status_flags_ambiguous(self, tmp_path: Path):
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="desktop")
        session = MCPSession(home=tmp_path, local=FakeComputer())
        dispatcher = MCPDispatcher(create_registry(), session)
        out = await dispatcher.call_tool("opendesk_status", {})
        text = out[0].text  # type: ignore[union-attr]
        # Status should flag ambiguity and tell the agent how to resolve.
        assert "multiple peers" in text.lower() or "ambiguous" in text.lower()
        assert "opendesk_use" in text

    @pytest.mark.asyncio
    async def test_explicit_local_overrides_implicit_default(self, tmp_path: Path):
        local = FakeComputer()
        remote = FakeComputer()
        session = _session_with_remote(tmp_path, local, remote, name="mini")
        computer, resolved = await session.resolve("local")
        assert computer is local and resolved == LOCAL

    @pytest.mark.asyncio
    async def test_explicit_use_peer_takes_precedence_over_implicit(self, tmp_path: Path):
        """If a user `use_peer('mini')`s then later pairs a second peer,
        the explicit choice stays in effect (source = 'explicit')."""
        local = FakeComputer()
        remote = FakeComputer()
        session = _session_with_remote(tmp_path, local, remote, name="mini")
        session.use_peer("mini")
        # Pair a second peer; the implicit-default fallback no longer applies.
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="desktop")
        name, source = session.effective_peer()
        assert name == "mini" and source == "explicit"


# ---------------------------------------------------------------------------
# Tool listing
# ---------------------------------------------------------------------------


class TestListTools:
    @pytest.mark.asyncio
    async def test_lists_computer_tools_with_peer_field(self, tmp_path: Path):
        dispatcher = MCPDispatcher(
            create_registry(), MCPSession(home=tmp_path, local=FakeComputer()),
        )
        entries = await dispatcher.list_tools()
        by_name = {e.name: e for e in entries}

        # Computer-use tool: should have `peer` in its schema's properties.
        screenshot = by_name["screenshot"]
        assert "peer" in screenshot.schema["properties"]
        assert "peer" in screenshot.description.lower()

        # Local-only tool: no `peer` field.
        assert "learn" in by_name
        assert "peer" not in by_name["learn"].schema.get("properties", {})

    @pytest.mark.asyncio
    async def test_admin_tools_listed(self, tmp_path: Path):
        dispatcher = MCPDispatcher(
            create_registry(), MCPSession(home=tmp_path, local=FakeComputer()),
        )
        names = {e.name for e in await dispatcher.list_tools()}
        for admin in (
            "opendesk_peers", "opendesk_discover", "opendesk_use",
            "opendesk_status", "opendesk_capabilities", "opendesk_disconnect",
        ):
            assert admin in names, f"missing {admin}"


# ---------------------------------------------------------------------------
# Computer-tool routing
# ---------------------------------------------------------------------------


class TestComputerToolRouting:
    @pytest.mark.asyncio
    async def test_default_routes_to_local(self, tmp_path: Path):
        local = FakeComputer()
        dispatcher = MCPDispatcher(
            create_registry(), MCPSession(home=tmp_path, local=local),
        )
        result = await dispatcher.call_tool("clipboard", {"action": "write", "text": "hi"})
        assert any(isinstance(r, TextResult) for r in result)
        assert any(c[0] == "clipboard_write" for c in local.calls)

    @pytest.mark.asyncio
    async def test_explicit_peer_routes_to_remote(self, tmp_path: Path):
        local = FakeComputer()
        remote = FakeComputer()
        session = _session_with_remote(tmp_path, local, remote, name="mini")
        dispatcher = MCPDispatcher(create_registry(), session)

        result = await dispatcher.call_tool(
            "clipboard", {"action": "write", "text": "remote-hi", "peer": "mini"},
        )
        # Output prefixed with "[on mini]"
        text = "\n".join(r.text for r in result if isinstance(r, TextResult))
        assert "[on mini]" in text
        # Remote received the write; local did not.
        assert any(c[0] == "clipboard_write" for c in remote.calls)
        assert not any(c[0] == "clipboard_write" for c in local.calls)

    @pytest.mark.asyncio
    async def test_default_peer_routes_to_remote(self, tmp_path: Path):
        local = FakeComputer()
        remote = FakeComputer()
        session = _session_with_remote(tmp_path, local, remote, name="mini")
        session.use_peer("mini")
        dispatcher = MCPDispatcher(create_registry(), session)

        await dispatcher.call_tool("clipboard", {"action": "write", "text": "x"})
        assert any(c[0] == "clipboard_write" for c in remote.calls)
        assert not any(c[0] == "clipboard_write" for c in local.calls)

    @pytest.mark.asyncio
    async def test_explicit_local_overrides_default(self, tmp_path: Path):
        local = FakeComputer()
        remote = FakeComputer()
        session = _session_with_remote(tmp_path, local, remote, name="mini")
        session.use_peer("mini")
        dispatcher = MCPDispatcher(create_registry(), session)

        await dispatcher.call_tool(
            "clipboard", {"action": "write", "text": "back", "peer": "local"},
        )
        assert any(c[0] == "clipboard_write" for c in local.calls)
        # remote is untouched
        assert not any(c[0] == "clipboard_write" for c in remote.calls)

    @pytest.mark.asyncio
    async def test_unknown_peer_returns_error(self, tmp_path: Path):
        dispatcher = MCPDispatcher(
            create_registry(), MCPSession(home=tmp_path, local=FakeComputer()),
        )
        result = await dispatcher.call_tool(
            "clipboard", {"action": "read", "peer": "doesnotexist"},
        )
        text = result[0].text  # type: ignore[union-attr]
        assert "ERROR" in text
        assert "doesnotexist" in text


# ---------------------------------------------------------------------------
# Admin tool dispatch
# ---------------------------------------------------------------------------


class TestAdminTools:
    @pytest.mark.asyncio
    async def test_peers_lists_local_and_trusted(self, tmp_path: Path):
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="desktop")
        session = MCPSession(home=tmp_path, local=FakeComputer())
        dispatcher = MCPDispatcher(create_registry(), session)
        out = await dispatcher.call_tool("opendesk_peers", {})
        text = out[0].text  # type: ignore[union-attr]
        assert "local" in text
        assert "mini" in text
        assert "desktop" in text

    @pytest.mark.asyncio
    async def test_use_then_status_reflects_default(self, tmp_path: Path):
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        session = MCPSession(home=tmp_path, local=FakeComputer())
        dispatcher = MCPDispatcher(create_registry(), session)

        await dispatcher.call_tool("opendesk_use", {"peer": "mini"})
        out = await dispatcher.call_tool("opendesk_status", {})
        assert "mini" in out[0].text  # type: ignore[union-attr]
        assert session.current_peer == "mini"

    @pytest.mark.asyncio
    async def test_use_local_reverts(self, tmp_path: Path):
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        session = MCPSession(home=tmp_path, local=FakeComputer())
        dispatcher = MCPDispatcher(create_registry(), session)

        session.use_peer("mini")
        await dispatcher.call_tool("opendesk_use", {"peer": "local"})
        assert session.current_peer is None

    @pytest.mark.asyncio
    async def test_capabilities_for_local(self, tmp_path: Path):
        session = MCPSession(home=tmp_path, local=FakeComputer())
        dispatcher = MCPDispatcher(create_registry(), session)
        out = await dispatcher.call_tool("opendesk_capabilities", {})
        text = out[0].text  # type: ignore[union-attr]
        assert "fake" in text
        assert "display.capture" in text

    @pytest.mark.asyncio
    async def test_disconnect_specific_peer(self, tmp_path: Path):
        local = FakeComputer()
        remote = FakeComputer()
        session = _session_with_remote(tmp_path, local, remote, name="mini")
        dispatcher = MCPDispatcher(create_registry(), session)

        # Force a connection by routing a call there.
        await dispatcher.call_tool("clipboard", {"action": "read", "peer": "mini"})
        assert "mini" in session.active_peer_names()

        await dispatcher.call_tool("opendesk_disconnect", {"peer": "mini"})
        assert "mini" not in session.active_peer_names()


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _session_with_remote(
    tmp_path: Path, local: FakeComputer, remote: FakeComputer, *, name: str,
) -> MCPSession:
    """Build a session where `name` resolves to *remote* without doing real I/O."""
    TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name=name)
    session = MCPSession(home=tmp_path, local=local)
    # Pre-seed the connection cache so resolve() returns the FakeComputer
    # without trying to discover + connect.
    session._connections[name] = remote  # type: ignore[attr-defined]
    return session
