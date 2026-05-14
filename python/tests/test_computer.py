"""Tests for the :class:`Computer` abstraction and its integration with tools.

These tests use an in-memory :class:`FakeComputer` for tools that perform
state-changing actions, so they run hermetically without touching a real
keyboard, mouse, or filesystem.
"""

from __future__ import annotations

from typing import Any, AsyncIterator, Optional

import pytest

from opendesk.computer import (
    Capability,
    CapabilityManifest,
    ClipboardContents,
    ClipboardEntry,
    CompletedCommand,
    Computer,
    Display,
    DisplayFrame,
    Environment,
    FileEntry,
    InputEvent,
    KeyEvent,
    LocalComputer,
    Notification,
    Pixmap,
    PixmapFormat,
    Point,
    PointerEvent,
    Process,
    Rect,
    TextInput,
    UIElement,
    Window,
)
from opendesk.tools.base import ToolContext, allow_all_context


from tests._fakes import FakeComputer


# ---------------------------------------------------------------------------
# LocalComputer / ToolContext
# ---------------------------------------------------------------------------


class TestLocalComputer:
    def test_capabilities_lists_core_set(self):
        c = LocalComputer()
        manifest = c.capabilities()
        for cap in (
            Capability.DISPLAY_CAPTURE, Capability.INPUT_POINTER,
            Capability.INPUT_KEYBOARD, Capability.CLIPBOARD_READ,
            Capability.UI_TREE, Capability.APP_LIFECYCLE,
        ):
            assert manifest.has(cap), f"missing {cap.value}"

    def test_backend_string(self):
        manifest = LocalComputer().capabilities()
        assert manifest.backend.startswith("local/")


class TestToolContextComputer:
    def test_default_context_has_local_computer(self):
        ctx = allow_all_context()
        assert isinstance(ctx.computer, LocalComputer)

    def test_context_accepts_custom_computer(self):
        fake = FakeComputer()
        ctx = ToolContext(computer=fake)
        assert ctx.computer is fake


# ---------------------------------------------------------------------------
# Tools route through ctx.computer — the load-bearing rewire invariant
# ---------------------------------------------------------------------------


class TestToolsRouteThroughComputer:
    @pytest.mark.asyncio
    async def test_clipboard_write_calls_computer(self):
        from opendesk.tools.clipboard import ClipboardTool
        fake = FakeComputer()
        ctx = ToolContext(computer=fake)
        tool = ClipboardTool()
        result = await tool.execute(ctx, ClipboardTool.Params(action="write", text="hello"))
        assert not result.error
        assert ("clipboard_write", {"text": "hello"}) in fake.calls

    @pytest.mark.asyncio
    async def test_clipboard_read_calls_computer(self):
        from opendesk.tools.clipboard import ClipboardTool
        fake = FakeComputer()
        fake._clipboard = "from-fake"
        ctx = ToolContext(computer=fake)
        tool = ClipboardTool()
        result = await tool.execute(ctx, ClipboardTool.Params(action="read"))
        assert not result.error
        assert "from-fake" in result.output

    @pytest.mark.asyncio
    async def test_keyboard_type_calls_computer_text(self):
        from opendesk.tools.keyboard import KeyboardTool
        fake = FakeComputer()
        ctx = ToolContext(computer=fake)
        tool = KeyboardTool()
        result = await tool.execute(
            ctx, KeyboardTool.Params(action="type", text="hi there", settle_ms=0)
        )
        assert not result.error
        assert ("text", {"text": "hi there"}) in fake.calls

    @pytest.mark.asyncio
    async def test_keyboard_hotkey_emits_key_events(self):
        from opendesk.tools.keyboard import KeyboardTool
        fake = FakeComputer()
        ctx = ToolContext(computer=fake)
        tool = KeyboardTool()
        result = await tool.execute(
            ctx, KeyboardTool.Params(action="hotkey", keys=["ctrl", "c"], settle_ms=0),
        )
        assert not result.error
        key_calls = [c for c in fake.calls if c[0] == "key"]
        assert len(key_calls) == 4  # ctrl down, c down, c up, ctrl up

    @pytest.mark.asyncio
    async def test_mouse_click_emits_pointer_events(self):
        from opendesk.tools.mouse import MouseTool
        fake = FakeComputer()
        ctx = ToolContext(computer=fake)
        tool = MouseTool()
        result = await tool.execute(
            ctx,
            MouseTool.Params(
                action="click", x=100, y=200,
                image_width=1920, image_height=1080,
                duration=0, settle_ms=0,
            ),
        )
        assert not result.error
        pointer_calls = [c for c in fake.calls if c[0] == "pointer"]
        assert len(pointer_calls) >= 3  # move + down + up

    @pytest.mark.asyncio
    async def test_app_open_calls_computer(self):
        from opendesk.tools.app import AppTool
        fake = FakeComputer()
        ctx = ToolContext(computer=fake)
        tool = AppTool()
        result = await tool.execute(ctx, AppTool.Params(action="open", name="TextEdit"))
        assert not result.error
        assert ("open_app", {"name": "TextEdit"}) in fake.calls

    @pytest.mark.asyncio
    async def test_app_list_calls_computer(self):
        from opendesk.tools.app import AppTool
        fake = FakeComputer()
        ctx = ToolContext(computer=fake)
        tool = AppTool()
        result = await tool.execute(ctx, AppTool.Params(action="list"))
        assert not result.error
        assert "Fake.app" in result.output

    @pytest.mark.asyncio
    async def test_ui_get_tree_calls_computer(self):
        from opendesk.tools.ui import UITool
        fake = FakeComputer()
        ctx = ToolContext(computer=fake)
        tool = UITool()
        result = await tool.execute(ctx, UITool.Params(action="get_tree", app="TextEdit"))
        assert not result.error
        assert "Save" in result.output

    @pytest.mark.asyncio
    async def test_ui_click_prefers_perform_ui_action(self):
        """When the backend advertises UI_ACTIONS, click should route through it."""
        from opendesk.tools.ui import UITool
        fake = FakeComputer()
        ctx = ToolContext(computer=fake)
        tool = UITool()
        result = await tool.execute(
            ctx, UITool.Params(action="click", app="TextEdit", title="Save"),
        )
        assert not result.error, result.output
        a11y_calls = [c for c in fake.calls if c[0] == "perform_ui_action"]
        pointer_calls = [c for c in fake.calls if c[0] == "pointer"]
        assert a11y_calls, "expected native a11y invocation"
        assert not pointer_calls, "should not fall back to pointer events when a11y works"
        assert a11y_calls[0][1]["element"].name == "Save"
        assert a11y_calls[0][1]["app"] == "TextEdit"

    @pytest.mark.asyncio
    async def test_ui_click_menu_uses_menu_path(self):
        from opendesk.tools.ui import UITool
        fake = FakeComputer()
        ctx = ToolContext(computer=fake)
        tool = UITool()
        result = await tool.execute(
            ctx,
            UITool.Params(action="click_menu", app="TextEdit", menu="File", menu_item="Save"),
        )
        assert not result.error, result.output
        a11y_calls = [c for c in fake.calls if c[0] == "perform_ui_action"]
        assert a11y_calls, "expected native a11y menu invocation"
        synth = a11y_calls[0][1]["element"]
        assert synth.metadata.get("menu_path") == ["File", "Save"]

    @pytest.mark.asyncio
    async def test_ui_click_falls_back_to_pointer_when_no_a11y(self):
        from opendesk.computer.base import CapabilityUnsupported
        from opendesk.tools.ui import UITool

        class NoActionsComputer(FakeComputer):
            def capabilities(self) -> CapabilityManifest:
                caps = set(Capability) - {Capability.UI_ACTIONS}
                return CapabilityManifest(capabilities=caps, backend="fake-no-actions")

            async def perform_ui_action(self, element, action="click", *, app=None):
                raise CapabilityUnsupported(Capability.UI_ACTIONS, backend="fake-no-actions")

        fake = NoActionsComputer()
        ctx = ToolContext(computer=fake)
        tool = UITool()
        result = await tool.execute(
            ctx, UITool.Params(action="click", app="TextEdit", title="Save"),
        )
        assert not result.error, result.output
        pointer_calls = [c for c in fake.calls if c[0] == "pointer"]
        assert pointer_calls, "expected bounds-center pointer fallback"


# ---------------------------------------------------------------------------
# Types — round-trip and coordinate translation
# ---------------------------------------------------------------------------


class TestTypes:
    def test_pixmap_coordinate_translation(self):
        # A 1920px-wide image of a 1440px-wide logical screen (Retina downscale).
        pm = Pixmap(
            data=b"x", format=PixmapFormat.PNG,
            width=1920, height=1200,
            logical_width=1440, logical_height=900,
        )
        # Point at the centre of the captured image translates to logical centre.
        p_logical = pm.to_logical(Point(x=960, y=600))
        assert abs(p_logical.x - 720) < 0.5
        assert abs(p_logical.y - 450) < 0.5
        # And back.
        p_pix = pm.to_pixel(p_logical)
        assert abs(p_pix.x - 960) < 0.5
        assert abs(p_pix.y - 600) < 0.5

    def test_rect_center(self):
        r = Rect(x=10, y=20, width=100, height=50)
        c = r.center
        assert c.x == 60 and c.y == 45

    def test_clipboard_roundtrip(self):
        e = ClipboardEntry.from_text("hello")
        assert e.as_text() == "hello"
        contents = ClipboardContents(entries=[e])
        assert contents.text() == "hello"
