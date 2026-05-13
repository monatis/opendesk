"""End-to-end tests for :class:`RemoteComputer` over :class:`LoopbackConnection`.

The wiring under test::

    FakeComputer  ──►  ComputerDispatcher  ──►  Peer(server)
                                                    │
                                       LoopbackConnection
                                                    │
    RemoteComputer  ◄──  Peer(client)               ▼

Every call through ``RemoteComputer`` exercises the full protocol path
(serialise → msgpack → wire → msgpack → deserialise → dispatch) but in-process,
so failures point at the protocol layer or the dispatcher rather than at
networking.
"""

from __future__ import annotations

import asyncio
from typing import AsyncIterator

import pytest

from opendesk.computer import (
    Capability,
    CapabilityUnsupported,
    ClipboardContents,
    ClipboardEntry,
    ComputerDispatcher,
    KeyAction,
    KeyEvent,
    Modifier,
    PointerAction,
    PointerButton,
    PointerEvent,
    Point,
    Rect,
    RemoteComputer,
    TextInput,
    UIElement,
)
from opendesk.protocol import LoopbackConnection, Peer

from tests._fakes import FakeComputer


# ---------------------------------------------------------------------------
# Fixture helpers
# ---------------------------------------------------------------------------


async def _build_pair() -> tuple[FakeComputer, RemoteComputer, Peer]:
    """Spin up a server peer + RemoteComputer wired through a loopback link."""
    fake = FakeComputer()
    a, b = LoopbackConnection.pair()

    server_peer = Peer(b, role="server", dispatcher=ComputerDispatcher(fake))

    async def _server_hello():
        await server_peer.hello(fake.capabilities().model_dump())
        server_peer.start()

    server_task = asyncio.create_task(_server_hello())
    remote = await RemoteComputer.connect(a)
    await server_task
    return fake, remote, server_peer


async def _teardown(remote: RemoteComputer, server_peer: Peer) -> None:
    await remote.aclose()
    await server_peer.aclose()


# ---------------------------------------------------------------------------
# Handshake / capabilities
# ---------------------------------------------------------------------------


class TestHello:
    @pytest.mark.asyncio
    async def test_capabilities_from_hello_no_round_trip(self):
        fake, remote, server = await _build_pair()
        try:
            assert remote.capabilities().has(Capability.DISPLAY_CAPTURE)
            assert remote.capabilities().backend == "fake"
            # No fake call was recorded — capabilities() served from the cache.
            assert all(c[0] != "capabilities" for c in fake.calls)
        finally:
            await _teardown(remote, server)


# ---------------------------------------------------------------------------
# Display / observation
# ---------------------------------------------------------------------------


class TestObservation:
    @pytest.mark.asyncio
    async def test_cursor_position(self):
        fake, remote, server = await _build_pair()
        try:
            pos = await remote.cursor_position()
            assert pos.x == 50 and pos.y == 60
        finally:
            await _teardown(remote, server)

    @pytest.mark.asyncio
    async def test_displays_list_round_trip(self):
        fake, remote, server = await _build_pair()
        try:
            displays = await remote.displays()
            assert len(displays) == 1
            assert displays[0].id == "1"
            assert displays[0].bounds.width == 1920
        finally:
            await _teardown(remote, server)

    @pytest.mark.asyncio
    async def test_capture_preserves_raw_bytes(self):
        """The binary path that motivated the no-base64 decision."""
        fake, remote, server = await _build_pair()
        try:
            pixmap = await remote.capture(region=Rect(x=0, y=0, width=10, height=10))
            assert pixmap.data.startswith(b"\x89PNGfake")
            assert pixmap.width == 100
            assert isinstance(pixmap.data, bytes)
            # The dispatcher passed Rect through correctly.
            cap_calls = [c for c in fake.calls if c[0] == "capture"]
            assert cap_calls and cap_calls[0][1]["region"].width == 10
        finally:
            await _teardown(remote, server)

    @pytest.mark.asyncio
    async def test_environment(self):
        fake, remote, server = await _build_pair()
        try:
            env = await remote.environment()
            assert env.os == "fake"
            assert env.displays[0].bounds.width == 1920
        finally:
            await _teardown(remote, server)


# ---------------------------------------------------------------------------
# Input / action
# ---------------------------------------------------------------------------


class TestActions:
    @pytest.mark.asyncio
    async def test_pointer_event_with_modifiers(self):
        fake, remote, server = await _build_pair()
        try:
            evt = PointerEvent(
                action=PointerAction.DOWN,
                point=Point(x=100, y=200),
                button=PointerButton.RIGHT,
                modifiers=[Modifier.SHIFT, Modifier.META],
            )
            await remote.pointer(evt)

            recv = [c for c in fake.calls if c[0] == "pointer"]
            assert recv, "expected pointer event to reach FakeComputer"
            received_event = recv[0][1]["event"]
            assert received_event.button == PointerButton.RIGHT
            assert Modifier.SHIFT in received_event.modifiers
            assert Modifier.META in received_event.modifiers
        finally:
            await _teardown(remote, server)

    @pytest.mark.asyncio
    async def test_key_event(self):
        fake, remote, server = await _build_pair()
        try:
            await remote.key(KeyEvent(action=KeyAction.DOWN, keysym="enter"))
            assert any(c[0] == "key" for c in fake.calls)
        finally:
            await _teardown(remote, server)

    @pytest.mark.asyncio
    async def test_text_with_unicode(self):
        fake, remote, server = await _build_pair()
        try:
            await remote.text(TextInput(text="hello 👋 ünïcödé"))
            text_calls = [c for c in fake.calls if c[0] == "text"]
            assert text_calls and text_calls[0][1]["text"] == "hello 👋 ünïcödé"
        finally:
            await _teardown(remote, server)

    @pytest.mark.asyncio
    async def test_app_open(self):
        fake, remote, server = await _build_pair()
        try:
            await remote.open_app("TextEdit")
            assert ("open_app", {"name": "TextEdit"}) in fake.calls
        finally:
            await _teardown(remote, server)

    @pytest.mark.asyncio
    async def test_list_apps(self):
        fake, remote, server = await _build_pair()
        try:
            apps = await remote.list_apps()
            assert apps == ["Fake.app"]
        finally:
            await _teardown(remote, server)


# ---------------------------------------------------------------------------
# Clipboard / round-trip with bytes
# ---------------------------------------------------------------------------


class TestClipboard:
    @pytest.mark.asyncio
    async def test_write_then_read(self):
        fake, remote, server = await _build_pair()
        try:
            await remote.clipboard_set_text("on the clipboard")
            text = await remote.clipboard_text()
            assert text == "on the clipboard"
        finally:
            await _teardown(remote, server)

    @pytest.mark.asyncio
    async def test_multi_entry_clipboard_round_trip(self):
        fake, remote, server = await _build_pair()
        try:
            contents = ClipboardContents(entries=[
                ClipboardEntry.from_text("hi"),
                ClipboardEntry(mime_type="application/octet-stream", data=b"\x00\xff\x42"),
            ])
            await remote.clipboard_write(contents)
            wr = [c for c in fake.calls if c[0] == "clipboard_write"]
            assert wr and wr[0][1]["text"] == "hi"
        finally:
            await _teardown(remote, server)


# ---------------------------------------------------------------------------
# UI tree / actions
# ---------------------------------------------------------------------------


class TestUI:
    @pytest.mark.asyncio
    async def test_ui_tree_with_nested_bounds(self):
        fake, remote, server = await _build_pair()
        try:
            tree = await remote.ui_tree(app="TextEdit")
            assert tree.name == "TextEdit"
            assert tree.children[0].role == "button"
            assert tree.children[0].bounds is not None
            assert tree.children[0].bounds.width == 80
        finally:
            await _teardown(remote, server)

    @pytest.mark.asyncio
    async def test_perform_ui_action_translates(self):
        fake, remote, server = await _build_pair()
        try:
            elem = UIElement(
                role="button", name="Save",
                bounds=Rect(x=10, y=20, width=80, height=30),
                actions=["click"],
                metadata={"menu_path": ["File", "Save"]},
            )
            await remote.perform_ui_action(elem, "click", app="TextEdit")

            recv = [c for c in fake.calls if c[0] == "perform_ui_action"]
            assert recv
            received = recv[0][1]["element"]
            assert received.name == "Save"
            assert received.metadata.get("menu_path") == ["File", "Save"]
            assert recv[0][1]["app"] == "TextEdit"
        finally:
            await _teardown(remote, server)


# ---------------------------------------------------------------------------
# Streaming
# ---------------------------------------------------------------------------


class TestStream:
    @pytest.mark.asyncio
    async def test_display_subscribe_stream(self):
        fake, remote, server = await _build_pair()
        try:
            frames = []
            async for frame in remote.subscribe_display(fps=30):
                frames.append(frame)
            assert len(frames) == 3
            assert frames[0].pixmap.data == b"frame" + bytes([0])
            assert frames[0].keyframe is True
            assert frames[1].keyframe is False
            assert frames[2].pixmap.data == b"frame" + bytes([2])
        finally:
            await _teardown(remote, server)


# ---------------------------------------------------------------------------
# Errors round-trip back into the right Python exception type
# ---------------------------------------------------------------------------


class TestErrorMapping:
    @pytest.mark.asyncio
    async def test_file_not_found_translates(self):
        fake, remote, server = await _build_pair()
        try:
            # FakeComputer.stat raises FileNotFoundError.
            with pytest.raises(Exception) as exc_info:
                await remote.stat("/missing")
            # Peer maps "not_found" → ProtocolError; we don't auto-translate
            # back to FileNotFoundError, but the wire code is preserved.
            assert "not_found" in str(exc_info.value)
        finally:
            await _teardown(remote, server)

    @pytest.mark.asyncio
    async def test_capability_unsupported_translates_back(self):
        """When the server raises CapabilityUnsupported, the client sees it."""
        from opendesk.computer.base import CapabilityUnsupported as ServerCU

        class StubbornComputer(FakeComputer):
            async def lock_screen(self) -> None:
                raise ServerCU(Capability.POWER_LOCK, backend="fake")

        a, b = LoopbackConnection.pair()
        server_peer = Peer(b, role="server", dispatcher=ComputerDispatcher(StubbornComputer()))

        async def _server_hello():
            await server_peer.hello({})
            server_peer.start()

        server_task = asyncio.create_task(_server_hello())
        remote = await RemoteComputer.connect(a)
        await server_task

        try:
            with pytest.raises(CapabilityUnsupported) as exc_info:
                await remote.lock_screen()
            assert exc_info.value.capability == Capability.POWER_LOCK
            assert exc_info.value.backend == "fake"
        finally:
            await remote.aclose()
            await server_peer.aclose()


# ---------------------------------------------------------------------------
# THE load-bearing test — existing tools work transparently against a
# remote computer.  Nothing in the tools themselves changed.
# ---------------------------------------------------------------------------


class TestToolsAgainstRemoteComputer:
    @pytest.mark.asyncio
    async def test_clipboard_tool_works_with_remote(self):
        from opendesk.tools.base import ToolContext
        from opendesk.tools.clipboard import ClipboardTool

        fake, remote, server = await _build_pair()
        try:
            ctx = ToolContext(computer=remote)
            tool = ClipboardTool()

            await tool.execute(ctx, ClipboardTool.Params(action="write", text="remote-hello"))
            wr = [c for c in fake.calls if c[0] == "clipboard_write"]
            assert wr and wr[0][1]["text"] == "remote-hello"

            result = await tool.execute(ctx, ClipboardTool.Params(action="read"))
            assert not result.error
            assert "remote-hello" in result.output
        finally:
            await _teardown(remote, server)

    @pytest.mark.asyncio
    async def test_ui_tool_works_with_remote(self):
        from opendesk.tools.base import ToolContext
        from opendesk.tools.ui import UITool

        fake, remote, server = await _build_pair()
        try:
            ctx = ToolContext(computer=remote)
            tool = UITool()
            result = await tool.execute(
                ctx, UITool.Params(action="click", app="TextEdit", title="Save"),
            )
            assert not result.error, result.output

            # Because the FakeComputer advertises UI_ACTIONS, the UI tool
            # should route through perform_ui_action — verifying the whole
            # ABC method chain works through the wire.
            a11y = [c for c in fake.calls if c[0] == "perform_ui_action"]
            assert a11y, "expected perform_ui_action to be invoked remotely"
        finally:
            await _teardown(remote, server)
