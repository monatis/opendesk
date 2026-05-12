"""Server-side dispatcher — turns inbound protocol calls into Computer methods.

A :class:`ComputerDispatcher` wraps any :class:`~opendesk.computer.Computer`
and implements the :class:`~opendesk.protocol.Dispatcher` Protocol so that a
:class:`~opendesk.protocol.Peer` can route REQ frames to it.

Wire boundary
-------------
Every protocol method's params and result are plain dicts.  This module owns
the boundary — it deserialises params into the right Pydantic models before
calling the Computer, and serialises the Computer's return value back into a
dict before handing it to the peer to encode.  Wire-method name → Computer
method routing is implemented as plain ``if`` chains for readability; there
are 35-ish methods total.
"""

from __future__ import annotations

import asyncio
from typing import Any, AsyncIterator, Optional

from opendesk.computer.base import CapabilityUnsupported, Computer
from opendesk.computer.types import (
    Capability,
    ClipboardContents,
    KeyEvent,
    PointerEvent,
    Rect,
    TextInput,
    UIElement,
)


class ComputerDispatcher:
    """Routes protocol method calls to a wrapped :class:`Computer`."""

    def __init__(self, computer: Computer) -> None:
        self._computer = computer

    # ------------------------------------------------------------------
    # Dispatcher protocol
    # ------------------------------------------------------------------

    async def call(self, method: str, params: dict[str, Any]) -> Optional[dict[str, Any]]:
        handler = _UNARY_DISPATCH.get(method)
        if handler is None:
            raise CapabilityUnsupported(
                Capability.UI_TREE if method.startswith("ui.") else Capability.DISPLAY_CAPTURE,
                backend=f"server (unknown method {method!r})",
            )
        return await handler(self._computer, params)

    def stream(
        self, method: str, params: dict[str, Any],
    ) -> AsyncIterator[dict[str, Any]]:
        handler = _STREAM_DISPATCH.get(method)
        if handler is None:
            return _unsupported_stream(method)
        return handler(self._computer, params)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


async def _unsupported_stream(method: str) -> AsyncIterator[dict[str, Any]]:
    raise CapabilityUnsupported(
        Capability.DISPLAY_STREAM, backend=f"server (unknown stream {method!r})",
    )
    yield  # pragma: no cover  (makes this an async generator)


def _opt_rect(value: Any) -> Optional[Rect]:
    if value is None:
        return None
    return Rect.model_validate(value)


def _dump(model: Any) -> dict[str, Any]:
    return model.model_dump(mode="python")


def _dump_list(models: list[Any]) -> list[dict[str, Any]]:
    return [m.model_dump(mode="python") for m in models]


# ---------------------------------------------------------------------------
# Unary handlers — one per protocol method
# ---------------------------------------------------------------------------


async def _system_capabilities(c: Computer, _p: dict) -> dict:
    return _dump(c.capabilities())


async def _system_environment(c: Computer, _p: dict) -> dict:
    return _dump(await c.environment())


async def _display_displays(c: Computer, _p: dict) -> dict:
    return {"items": _dump_list(await c.displays())}


async def _display_capture(c: Computer, p: dict) -> dict:
    pixmap = await c.capture(
        display_id=p.get("display_id"),
        region=_opt_rect(p.get("region")),
        downscale=p.get("downscale", True),
    )
    return _dump(pixmap)


async def _display_cursor(c: Computer, _p: dict) -> dict:
    return _dump(await c.cursor_position())


async def _input_pointer(c: Computer, p: dict) -> None:
    await c.pointer(PointerEvent.model_validate(p["event"]))
    return None


async def _input_key(c: Computer, p: dict) -> None:
    await c.key(KeyEvent.model_validate(p["event"]))
    return None


async def _input_text(c: Computer, p: dict) -> None:
    await c.text(TextInput.model_validate(p["text_input"]))
    return None


async def _windows_list(c: Computer, _p: dict) -> dict:
    return {"items": _dump_list(await c.windows())}


async def _windows_focused(c: Computer, _p: dict) -> dict:
    w = await c.focused_window()
    return {"window": _dump(w) if w else None}


async def _windows_focus(c: Computer, p: dict) -> None:
    await c.focus_window(p["window_id"])
    return None


async def _windows_move(c: Computer, p: dict) -> None:
    await c.move_window(p["window_id"], Rect.model_validate(p["bounds"]))
    return None


async def _windows_close(c: Computer, p: dict) -> None:
    await c.close_window(p["window_id"])
    return None


async def _apps_open(c: Computer, p: dict) -> None:
    await c.open_app(p["name"])
    return None


async def _apps_close(c: Computer, p: dict) -> None:
    await c.close_app(p["name"])
    return None


async def _apps_focus(c: Computer, p: dict) -> None:
    await c.focus_app(p["name"])
    return None


async def _apps_list(c: Computer, _p: dict) -> dict:
    return {"items": await c.list_apps()}


async def _ui_tree(c: Computer, p: dict) -> dict:
    tree = await c.ui_tree(
        window_id=p.get("window_id"),
        app=p.get("app"),
        max_depth=p.get("max_depth", 8),
    )
    return _dump(tree)


async def _ui_action(c: Computer, p: dict) -> None:
    element = UIElement.model_validate(p["element"])
    await c.perform_ui_action(element, p.get("action", "click"), app=p.get("app"))
    return None


async def _clipboard_read(c: Computer, _p: dict) -> dict:
    return _dump(await c.clipboard_read())


async def _clipboard_write(c: Computer, p: dict) -> None:
    await c.clipboard_write(ClipboardContents.model_validate(p["contents"]))
    return None


async def _fs_read(c: Computer, p: dict) -> dict:
    return {"data": await c.read_file(p["path"])}


async def _fs_write(c: Computer, p: dict) -> None:
    await c.write_file(p["path"], p["data"])
    return None


async def _fs_list(c: Computer, p: dict) -> dict:
    return {"items": _dump_list(await c.list_dir(p["path"]))}


async def _fs_stat(c: Computer, p: dict) -> dict:
    return _dump(await c.stat(p["path"]))


async def _fs_delete(c: Computer, p: dict) -> None:
    await c.delete(p["path"])
    return None


async def _fs_move(c: Computer, p: dict) -> None:
    await c.move(p["src"], p["dst"])
    return None


async def _fs_mkdir(c: Computer, p: dict) -> None:
    await c.mkdir(p["path"], parents=p.get("parents", True))
    return None


async def _process_list(c: Computer, _p: dict) -> dict:
    return {"items": _dump_list(await c.processes())}


async def _process_shell(c: Computer, p: dict) -> dict:
    return _dump(await c.shell(
        p["command"], timeout=p.get("timeout"),
        cwd=p.get("cwd"), env=p.get("env"),
    ))


async def _process_exec(c: Computer, p: dict) -> dict:
    return _dump(await c.exec(
        p["argv"], timeout=p.get("timeout"),
        cwd=p.get("cwd"), env=p.get("env"), stdin=p.get("stdin"),
    ))


async def _power_lock(c: Computer, _p: dict) -> None:
    await c.lock_screen()
    return None


async def _notifications_list(c: Computer, _p: dict) -> dict:
    return {"items": _dump_list(await c.notifications())}


# ---------------------------------------------------------------------------
# Stream handlers
# ---------------------------------------------------------------------------


async def _display_subscribe(c: Computer, p: dict) -> AsyncIterator[dict]:
    async for frame in c.subscribe_display(
        display_id=p.get("display_id"),
        fps=p.get("fps", 30),
        region=_opt_rect(p.get("region")),
    ):
        yield _dump(frame)


async def _input_subscribe(c: Computer, p: dict) -> AsyncIterator[dict]:
    async for event in c.subscribe_input():
        yield _dump(event)


# ---------------------------------------------------------------------------
# Dispatch tables
# ---------------------------------------------------------------------------


_UNARY_DISPATCH: dict[str, Any] = {
    "system.capabilities": _system_capabilities,
    "system.environment": _system_environment,
    "display.displays": _display_displays,
    "display.capture": _display_capture,
    "display.cursor_position": _display_cursor,
    "input.pointer": _input_pointer,
    "input.key": _input_key,
    "input.text": _input_text,
    "windows.list": _windows_list,
    "windows.focused": _windows_focused,
    "windows.focus": _windows_focus,
    "windows.move": _windows_move,
    "windows.close": _windows_close,
    "apps.open": _apps_open,
    "apps.close": _apps_close,
    "apps.focus": _apps_focus,
    "apps.list": _apps_list,
    "ui.tree": _ui_tree,
    "ui.action": _ui_action,
    "clipboard.read": _clipboard_read,
    "clipboard.write": _clipboard_write,
    "fs.read": _fs_read,
    "fs.write": _fs_write,
    "fs.list": _fs_list,
    "fs.stat": _fs_stat,
    "fs.delete": _fs_delete,
    "fs.move": _fs_move,
    "fs.mkdir": _fs_mkdir,
    "process.list": _process_list,
    "process.shell": _process_shell,
    "process.exec": _process_exec,
    "power.lock": _power_lock,
    "notifications.list": _notifications_list,
}


_STREAM_DISPATCH: dict[str, Any] = {
    "display.subscribe": _display_subscribe,
    "input.subscribe": _input_subscribe,
}
