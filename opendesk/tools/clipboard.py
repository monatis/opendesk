"""ClipboardTool — read and write the system clipboard."""

from __future__ import annotations

import asyncio
import platform
import subprocess
from typing import Literal, Optional

from pydantic import Field

from opendesk.tools.base import Tool, ToolContext, ToolResult

_PLATFORM = platform.system()


class ClipboardTool(Tool):
    """Read or write the system clipboard."""

    name = "clipboard"
    description = (
        "Read the current clipboard text or write new text to the clipboard. "
        "Use 'read' to retrieve copied text; 'write' to place text on the clipboard."
    )

    class Params(Tool.Params):
        action: Literal["read", "write"] = Field(
            description="Clipboard action: read or write."
        )
        text: Optional[str] = Field(
            default=None,
            description="Text to place on the clipboard. Required for action='write'.",
        )

    async def execute(self, ctx: ToolContext, params: "ClipboardTool.Params") -> ToolResult:
        from opendesk.computer.sandbox import ActionType, get_sandbox

        arg_desc = (
            f"clipboard write: {(params.text or '')[:40]!r}"
            if params.action == "write"
            else "clipboard read"
        )
        await ctx.check_permission(
            tool="clipboard", argument=arg_desc,
            description=f"Clipboard: {arg_desc}",
        )

        sandbox = get_sandbox(ctx.session_id)
        loop = asyncio.get_event_loop()

        try:
            result_msg = await loop.run_in_executor(None, self._do_action, params)
        except Exception as exc:
            action_type = ActionType.CLIPBOARD_READ if params.action == "read" else ActionType.CLIPBOARD_WRITE
            await sandbox.record_action(action_type, {"action": params.action}, error=str(exc))
            return ToolResult(title="Clipboard error", output=str(exc), error=True)

        action_type = ActionType.CLIPBOARD_READ if params.action == "read" else ActionType.CLIPBOARD_WRITE
        await sandbox.record_action(
            action_type,
            {"action": params.action, "text_len": len(params.text or "")},
            result=result_msg[:200],
        )
        return ToolResult(title=f"Clipboard: {params.action}", output=result_msg)

    @staticmethod
    def _do_action(params: "ClipboardTool.Params") -> str:
        if params.action == "read":
            return _read_clipboard()
        if params.action == "write":
            if not params.text:
                raise ValueError("'text' is required for action='write'.")
            _write_clipboard(params.text)
            preview = params.text[:80] + ("..." if len(params.text) > 80 else "")
            return f"Clipboard set ({len(params.text)} chars): {preview!r}"
        raise ValueError(f"Unknown clipboard action: {params.action!r}")


def _read_clipboard() -> str:
    if _PLATFORM == "Darwin":
        r = subprocess.run(["pbpaste"], capture_output=True, timeout=5)
        if r.returncode != 0:
            raise RuntimeError(f"pbpaste failed: {r.stderr.decode(errors='replace')}")
        text = r.stdout.decode("utf-8", errors="replace")
        return text if text else "(clipboard is empty)"
    if _PLATFORM == "Linux":
        for cmd in (
            ["xclip", "-selection", "clipboard", "-o"],
            ["xsel", "--clipboard", "--output"],
        ):
            r = subprocess.run(cmd, capture_output=True, timeout=5)
            if r.returncode == 0:
                text = r.stdout.decode("utf-8", errors="replace")
                return text if text else "(clipboard is empty)"
        try:
            import pyperclip  # type: ignore[import-not-found]
            t = pyperclip.paste()
            return t if t else "(clipboard is empty)"
        except ImportError:
            raise RuntimeError("Clipboard read requires xclip, xsel, or pyperclip.")
    if _PLATFORM == "Windows":
        try:
            import pyperclip  # type: ignore[import-not-found]
        except ImportError:
            from opendesk.computer.deps import ensure_import
            pyperclip = ensure_import("pyperclip")
        t = pyperclip.paste()
        return t if t else "(clipboard is empty)"
    raise RuntimeError(f"Clipboard not supported on platform: {_PLATFORM!r}")


def _write_clipboard(text: str) -> None:
    if _PLATFORM == "Darwin":
        r = subprocess.run(["pbcopy"], input=text.encode("utf-8"), capture_output=True, timeout=5)
        if r.returncode != 0:
            raise RuntimeError(f"pbcopy failed: {r.stderr.decode(errors='replace')}")
        return
    if _PLATFORM == "Linux":
        for cmd in (["xclip", "-selection", "clipboard"], ["xsel", "--clipboard", "--input"]):
            r = subprocess.run(cmd, input=text.encode("utf-8"), capture_output=True, timeout=5)
            if r.returncode == 0:
                return
        try:
            import pyperclip  # type: ignore[import-not-found]
            pyperclip.copy(text)
            return
        except ImportError:
            raise RuntimeError("Clipboard write requires xclip, xsel, or pyperclip.")
    if _PLATFORM == "Windows":
        try:
            import pyperclip  # type: ignore[import-not-found]
        except ImportError:
            from opendesk.computer.deps import ensure_import
            pyperclip = ensure_import("pyperclip")
        pyperclip.copy(text)
        return
    raise RuntimeError(f"Clipboard not supported on platform: {_PLATFORM!r}")
