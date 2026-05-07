"""Tool registry — create and look up tools by name.

Usage::

    from opencua.registry import create_registry

    registry = create_registry()
    tool = registry.get("screenshot")
    result = await tool.execute(ctx, tool.parse_params({"marks": True}))
"""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from opencua.tools.base import Tool


class ToolRegistry:
    """A dict-like container of :class:`~opencua.tools.base.Tool` instances."""

    def __init__(self) -> None:
        self._tools: dict[str, "Tool"] = {}

    def register(self, tool: "Tool") -> None:
        """Add *tool* to the registry."""
        self._tools[tool.name] = tool

    def get(self, name: str) -> "Tool":
        """Return the tool with *name*, raising :class:`KeyError` if not found."""
        if name not in self._tools:
            raise KeyError(
                f"Tool {name!r} not found. Available: {list(self._tools)}"
            )
        return self._tools[name]

    def names(self) -> list[str]:
        """Return sorted list of registered tool names."""
        return sorted(self._tools)

    def tools(self) -> list["Tool"]:
        """Return all registered tools."""
        return list(self._tools.values())

    def __contains__(self, name: str) -> bool:
        return name in self._tools

    def __len__(self) -> int:
        return len(self._tools)

    def __repr__(self) -> str:
        return f"ToolRegistry({self.names()})"


def create_registry() -> ToolRegistry:
    """Create a :class:`ToolRegistry` populated with all built-in tools.

    Tools included
    --------------
    - ``screenshot`` — capture screen, SoM marks, cursor overlay, zoom, diff
    - ``mouse``      — click, scroll, drag, move, Retina-aware coordinate scaling
    - ``keyboard``   — type (Unicode), press, hotkey, hold
    - ``app``        — open / close / focus / list applications
    - ``ui``         — accessibility-based interaction (FIRST CHOICE over mouse)
    - ``clipboard``  — read / write system clipboard
    - ``ocr``        — extract text via pytesseract / Vision / WinRT
    """
    from opencua.tools.screenshot import ScreenshotTool
    from opencua.tools.mouse import MouseTool
    from opencua.tools.keyboard import KeyboardTool
    from opencua.tools.app import AppTool
    from opencua.tools.ui import UITool
    from opencua.tools.clipboard import ClipboardTool
    from opencua.tools.ocr import OCRTool

    registry = ToolRegistry()
    for tool_cls in (
        ScreenshotTool,
        MouseTool,
        KeyboardTool,
        AppTool,
        UITool,
        ClipboardTool,
        OCRTool,
    ):
        registry.register(tool_cls())

    return registry


def create_minimal_registry() -> ToolRegistry:
    """Like :func:`create_registry` but without heavy optional tools (OCR).

    Suitable for environments where pytesseract / Tesseract are not installed.
    """
    from opencua.tools.screenshot import ScreenshotTool
    from opencua.tools.mouse import MouseTool
    from opencua.tools.keyboard import KeyboardTool
    from opencua.tools.app import AppTool
    from opencua.tools.ui import UITool
    from opencua.tools.clipboard import ClipboardTool

    registry = ToolRegistry()
    for tool_cls in (ScreenshotTool, MouseTool, KeyboardTool, AppTool, UITool, ClipboardTool):
        registry.register(tool_cls())
    return registry
