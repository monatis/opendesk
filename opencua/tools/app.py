"""AppTool — open, close, focus, and list desktop applications."""

from __future__ import annotations

import asyncio
import platform
import subprocess
from typing import Literal, Optional

from pydantic import Field

from opencua.tools.base import Tool, ToolContext, ToolResult

_PLATFORM = platform.system()


class AppTool(Tool):
    """Open, close, focus, or list desktop applications."""

    name = "app"
    description = (
        "Interact with desktop applications: open an app by name, close it, "
        "bring it to the foreground, or list all currently running windows."
    )

    class Params(Tool.Params):
        action: Literal["open", "close", "focus", "list"] = Field(
            description="App action: open, close, focus, or list."
        )
        name: Optional[str] = Field(
            default=None,
            description=(
                "Application name (e.g. 'Terminal', 'Google Chrome', 'VS Code') "
                "or full executable path. Required for open/close/focus."
            ),
        )

    async def execute(self, ctx: ToolContext, params: "AppTool.Params") -> ToolResult:
        from opencua.computer.sandbox import ActionType, get_sandbox

        app_arg = params.name or "(list)"
        await ctx.check_permission(
            tool="app", argument=f"{params.action} {app_arg}",
            description=f"App control: {params.action} '{app_arg}'",
        )

        sandbox = get_sandbox(ctx.session_id)

        if params.action in ("open", "close", "focus") and params.name:
            if not sandbox.is_app_allowed(params.name):
                return ToolResult(
                    title="App action denied",
                    output=(
                        f"Application '{params.name}' is not in the allow-list. "
                        f"Allowed: {sandbox.allowed_apps or ['(all)']}"
                    ),
                    error=True,
                )

        action_map = {
            "open": ActionType.APP_OPEN, "close": ActionType.APP_CLOSE,
            "focus": ActionType.APP_FOCUS, "list": ActionType.APP_LIST,
        }

        try:
            loop = asyncio.get_event_loop()
            result_msg = await loop.run_in_executor(None, self._do_action, params)
        except Exception as exc:
            await sandbox.record_action(
                action_map.get(params.action, ActionType.APP_OPEN),
                {"action": params.action, "name": params.name}, error=str(exc),
            )
            return ToolResult(
                title="App error",
                output=f"App action '{params.action}' failed: {exc}",
                error=True,
            )

        await sandbox.record_action(
            action_map.get(params.action, ActionType.APP_OPEN),
            {"action": params.action, "name": params.name}, result=result_msg[:200],
        )

        return ToolResult(
            title=f"App: {params.action} '{params.name or ''}'",
            output=result_msg,
        )

    @staticmethod
    def _do_action(params: "AppTool.Params") -> str:
        if params.action == "list":
            return _list_windows()
        if not params.name:
            raise ValueError(f"name is required for action='{params.action}'.")
        if params.action == "open":
            return _open_app(params.name)
        if params.action == "close":
            return _close_app(params.name)
        if params.action == "focus":
            return _focus_app(params.name)
        raise ValueError(f"Unknown app action: {params.action!r}")


def _run(cmd: list, **kwargs):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=15, **kwargs)


def _open_app(name: str) -> str:
    import time
    if _PLATFORM == "Darwin":
        r = _run(["open", "-a", name])
        if r.returncode != 0:
            r2 = _run(["open", name])
            if r2.returncode != 0:
                raise RuntimeError(r.stderr.strip() or r2.stderr.strip())
        time.sleep(1.5)
        script = (
            f'tell application "{name}"\n'
            f'    activate\n'
            f'    try\n'
            f'        if (count of documents) = 0 then make new document\n'
            f'    end try\n'
            f'end tell'
        )
        _run(["osascript", "-e", script])
        time.sleep(0.5)
        return f"Opened '{name}' on macOS."
    if _PLATFORM == "Linux":
        try:
            subprocess.Popen([name], start_new_session=True)
        except FileNotFoundError:
            subprocess.Popen(["xdg-open", name], start_new_session=True)
        time.sleep(2.0)
        return f"Opened '{name}' on Linux."
    if _PLATFORM == "Windows":
        subprocess.Popen(["start", "", name], shell=True, start_new_session=True)
        time.sleep(2.0)
        return f"Opened '{name}' on Windows."
    raise RuntimeError(f"Unsupported platform: {_PLATFORM}")


def _close_app(name: str) -> str:
    if _PLATFORM == "Darwin":
        r = _run(["osascript", "-e", f'tell application "{name}" to quit'])
        if r.returncode != 0:
            raise RuntimeError(r.stderr.strip())
        return f"Quit '{name}' via AppleScript."
    if _PLATFORM == "Linux":
        r = _run(["pkill", "-f", name])
        if r.returncode not in (0, 1):
            raise RuntimeError(r.stderr.strip())
        return f"Sent SIGTERM to processes matching '{name}'."
    if _PLATFORM == "Windows":
        r = _run(["taskkill", "/IM", name, "/F"])
        if r.returncode != 0:
            raise RuntimeError(r.stderr.strip())
        return f"Terminated '{name}' on Windows."
    raise RuntimeError(f"Unsupported platform: {_PLATFORM}")


def _focus_app(name: str) -> str:
    if _PLATFORM == "Darwin":
        r = _run(["osascript", "-e", f'tell application "{name}" to activate'])
        if r.returncode != 0:
            raise RuntimeError(r.stderr.strip())
        return f"Focused '{name}' via AppleScript."
    if _PLATFORM == "Linux":
        r = _run(["wmctrl", "-a", name])
        if r.returncode != 0:
            r2 = _run(["xdotool", "search", "--name", name, "windowactivate"])
            if r2.returncode != 0:
                raise RuntimeError(f"wmctrl: {r.stderr.strip()} | xdotool: {r2.stderr.strip()}")
        return f"Focused window matching '{name}'."
    if _PLATFORM == "Windows":
        try:
            import pygetwindow as gw  # type: ignore[import-not-found]
            wins = gw.getWindowsWithTitle(name)
            if not wins:
                raise RuntimeError(f"No window found with title '{name}'.")
            wins[0].activate()
            return f"Focused '{wins[0].title}'."
        except ImportError as exc:
            raise RuntimeError("pygetwindow required: pip install pygetwindow") from exc
    raise RuntimeError(f"Unsupported platform: {_PLATFORM}")


def _list_windows() -> str:
    if _PLATFORM == "Darwin":
        script = (
            'tell application "System Events" to get the name of every process '
            'whose background only is false'
        )
        r = _run(["osascript", "-e", script])
        if r.returncode != 0:
            raise RuntimeError(r.stderr.strip())
        names = [n.strip() for n in r.stdout.strip().split(",") if n.strip()]
        return "Running applications:\n" + "\n".join(f"  * {n}" for n in names)
    if _PLATFORM == "Linux":
        r = _run(["wmctrl", "-l"])
        if r.returncode == 0:
            lines = [ln.strip() for ln in r.stdout.strip().splitlines() if ln.strip()]
            return f"Open windows ({len(lines)}):\n" + "\n".join(f"  * {ln}" for ln in lines)
        r2 = _run(["ps", "-eo", "comm="])
        if r2.returncode == 0:
            procs = sorted(set(r2.stdout.strip().splitlines()))
            return "Running processes:\n" + "\n".join(f"  * {p}" for p in procs[:50])
        raise RuntimeError("Could not list windows.")
    if _PLATFORM == "Windows":
        try:
            import pygetwindow as gw  # type: ignore[import-not-found]
            titles = [w.title for w in gw.getAllWindows() if w.title.strip()]
            return f"Open windows ({len(titles)}):\n" + "\n".join(f"  * {t}" for t in titles)
        except ImportError:
            r = _run(["tasklist", "/FO", "CSV", "/NH"])
            if r.returncode == 0:
                lines = r.stdout.strip().splitlines()[:30]
                return "Running processes:\n" + "\n".join(f"  * {l}" for l in lines)
            raise RuntimeError("Could not list windows on Windows.")
    raise RuntimeError(f"Unsupported platform: {_PLATFORM}")
