"""KeyboardTool — type text and press key combinations."""

from __future__ import annotations

import asyncio
from typing import List, Literal, Optional

from pydantic import Field

from opendesk.tools.base import Tool, ToolContext, ToolResult


def _pyautogui():
    try:
        import pyautogui  # type: ignore[import-not-found]
        return pyautogui
    except ImportError as exc:
        raise ImportError(
            "pyautogui is required for keyboard control: pip install 'opendesk[core]'"
        ) from exc


def _check_accessibility() -> None:
    import platform
    if platform.system() != "Darwin":
        return
    import subprocess
    r = subprocess.run(
        ["osascript", "-e", 'tell application "System Events" to get name of first process'],
        capture_output=True, text=True, timeout=5,
    )
    if r.returncode != 0 and "not allowed" in (r.stderr + r.stdout).lower():
        raise RuntimeError(
            "macOS Accessibility permission is required for keyboard control. "
            "Go to System Settings -> Privacy & Security -> Accessibility."
        )


def _type_text(pag: object, text: str, interval: float) -> None:
    """Type text using clipboard-paste for full Unicode support."""
    import platform
    import time
    sys_name = platform.system()

    if sys_name == "Darwin":
        import subprocess
        subprocess.run(["pbcopy"], input=text.encode("utf-8"), check=True)
        time.sleep(0.1)
        import pyautogui  # type: ignore[import-not-found]
        pyautogui.hotkey("command", "v")

    elif sys_name == "Linux":
        import subprocess
        pasted = False
        for clip_cmd, paste_keys in [
            (["xclip", "-selection", "clipboard"], ["ctrl", "v"]),
            (["xsel", "--clipboard", "--input"], ["ctrl", "v"]),
        ]:
            try:
                subprocess.run(clip_cmd, input=text.encode("utf-8"), check=True,
                               capture_output=True, timeout=5)
                time.sleep(0.05)
                import pyautogui  # type: ignore[import-not-found]
                pyautogui.hotkey(*paste_keys)
                pasted = True
                break
            except (FileNotFoundError, subprocess.CalledProcessError):
                continue
        if not pasted:
            try:
                import subprocess as sp
                sp.run(
                    ["xdotool", "type", "--clearmodifiers", "--delay",
                     str(int(interval * 1000)), "--", text],
                    check=True, timeout=30,
                )
            except (FileNotFoundError, subprocess.CalledProcessError) as exc:
                raise RuntimeError(
                    "Cannot type text on Linux: install xclip: sudo apt install xclip"
                ) from exc

    elif sys_name == "Windows":
        try:
            import pyperclip  # type: ignore[import-not-found]
        except ImportError:
            from opendesk.computer.deps import ensure_import
            pyperclip = ensure_import("pyperclip")
        pyperclip.copy(text)
        time.sleep(0.05)
        import pyautogui  # type: ignore[import-not-found]
        pyautogui.hotkey("ctrl", "v")

    else:
        import pyautogui  # type: ignore[import-not-found]
        pyautogui.typewrite(text, interval=interval)


class KeyboardTool(Tool):
    """Type text or press keyboard keys and shortcuts."""

    name = "keyboard"
    description = (
        "Simulate keyboard input: type a string of text, press a single key, "
        "or send a key combination (hotkey). Unicode text is fully supported."
    )

    class Params(Tool.Params):
        action: Literal["type", "press", "hotkey", "hold"] = Field(
            description=(
                "Keyboard action: type, press, hotkey, or hold."
            )
        )
        text: Optional[str] = Field(default=None, description="Text to type. Required for action='type'.")
        key: Optional[str] = Field(
            default=None,
            description="Key name to press/hold: 'enter', 'escape', 'tab', 'f5', etc.",
        )
        keys: Optional[List[str]] = Field(
            default=None,
            description="Key names for hotkey, e.g. ['ctrl','c'].",
        )
        interval: float = Field(default=0.02, description="Seconds between keystrokes (action='type').")
        settle_ms: int = Field(default=300, description="Milliseconds to wait after the action.")
        hold_duration: float = Field(default=1.0, description="Seconds to hold key for action='hold'.")

    async def execute(self, ctx: ToolContext, params: "KeyboardTool.Params") -> ToolResult:
        from opendesk.computer.sandbox import ActionType, get_sandbox

        if params.action == "type":
            arg_desc = f"type text: {(params.text or '')[:60]!r}"
        elif params.action == "press":
            arg_desc = f"press key: {params.key}"
        elif params.action == "hold":
            arg_desc = f"hold key: {params.key} for {params.hold_duration}s"
        else:
            arg_desc = f"hotkey: {'+'.join(params.keys or [])}"

        await ctx.check_permission(
            tool="keyboard", argument=arg_desc,
            description=f"Keyboard action -- {arg_desc}",
        )

        sandbox = get_sandbox(ctx.session_id)
        action_params = {"action": params.action}

        try:
            loop = asyncio.get_event_loop()
            result_msg = await loop.run_in_executor(None, self._do_action, params)
        except ImportError as exc:
            return ToolResult(title="Keyboard error", output=str(exc), error=True)
        except ValueError as exc:
            return ToolResult(title="Keyboard error", output=str(exc), error=True)
        except Exception as exc:
            await sandbox.record_action(ActionType.KEYBOARD_TYPE, action_params, error=str(exc))
            return ToolResult(title="Keyboard error", output=f"Keyboard action failed: {exc}", error=True)

        action_type_map = {
            "type": ActionType.KEYBOARD_TYPE,
            "press": ActionType.KEYBOARD_PRESS,
            "hotkey": ActionType.KEYBOARD_HOTKEY,
            "hold": ActionType.KEYBOARD_HOLD,
        }
        await sandbox.record_action(
            action_type_map.get(params.action, ActionType.KEYBOARD_TYPE),
            action_params, result=result_msg,
        )

        return ToolResult(title=f"Keyboard: {params.action}", output=result_msg)

    @staticmethod
    def _do_action(params: "KeyboardTool.Params") -> str:
        import time
        _check_accessibility()
        pag = _pyautogui()
        settle = params.settle_ms / 1000.0

        if params.action == "type":
            if not params.text:
                raise ValueError("text is required for action='type'.")
            _type_text(pag, params.text, params.interval)
            time.sleep(settle)
            preview = params.text[:40] + ("..." if len(params.text) > 40 else "")
            return f"Typed {len(params.text)} characters: {preview!r}"

        if params.action == "press":
            if not params.key:
                raise ValueError("key is required for action='press'.")
            pag.press(params.key)
            time.sleep(settle)
            return f"Pressed key: {params.key!r}"

        if params.action == "hotkey":
            if not params.keys:
                raise ValueError("keys list is required for action='hotkey'.")
            pag.hotkey(*params.keys)
            time.sleep(settle)
            return f"Pressed hotkey: {'+'.join(params.keys)}"

        if params.action == "hold":
            if not params.key:
                raise ValueError("key is required for action='hold'.")
            pag.keyDown(params.key)
            time.sleep(max(0.0, params.hold_duration))
            pag.keyUp(params.key)
            time.sleep(settle)
            return f"Held key {params.key!r} for {params.hold_duration:.2f}s then released."

        raise ValueError(f"Unknown keyboard action: {params.action!r}")
