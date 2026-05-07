"""MouseTool — control the mouse pointer."""

from __future__ import annotations

import asyncio
from typing import Literal, Optional

from pydantic import Field

from opencua.tools.base import Tool, ToolContext, ToolResult


def _pyautogui():
    try:
        import pyautogui  # type: ignore[import-not-found]
        return pyautogui
    except ImportError as exc:
        raise ImportError(
            "pyautogui is required for mouse control: pip install 'opencua[core]'"
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
            "macOS Accessibility permission is required for mouse control. "
            "Go to System Settings -> Privacy & Security -> Accessibility."
        )


class MouseTool(Tool):
    """Move, click, scroll, or drag the mouse pointer."""

    name = "mouse"
    description = (
        "Control the mouse: move to a position, click (left/right/middle), "
        "double-click, triple-click, scroll, or drag from one point to another. "
        "Always provide image_width and image_height from the screenshot tool "
        "output for correct Retina/HiDPI coordinate translation."
    )

    class Params(Tool.Params):
        action: Literal[
            "move", "click", "double_click", "triple_click",
            "right_click", "middle_click",
            "left_down", "left_up",
            "scroll", "drag",
            "cursor_position",
        ] = Field(
            description=(
                "Mouse action: move, click, double_click, triple_click, "
                "right_click, middle_click, left_down, left_up, scroll, drag, cursor_position"
            )
        )
        x: int = Field(default=0, description="X coordinate as it appears in the screenshot.")
        y: int = Field(default=0, description="Y coordinate as it appears in the screenshot.")
        end_x: Optional[int] = Field(default=None, description="Target X for drag action.")
        end_y: Optional[int] = Field(default=None, description="Target Y for drag action.")
        image_width: Optional[int] = Field(
            default=None,
            description=(
                "Width of the screenshot the coordinates were read from. "
                "ALWAYS provide this for correct Retina/HiDPI scaling."
            ),
        )
        image_height: Optional[int] = Field(
            default=None,
            description="Height of the screenshot. Provide alongside image_width.",
        )
        direction: Optional[Literal["up", "down", "left", "right"]] = Field(
            default=None,
            description="Scroll direction. Used with action='scroll'.",
        )
        amount: int = Field(default=3, description="Scroll amount in clicks.")
        duration: float = Field(default=0.25, description="Movement duration in seconds.")
        settle_ms: int = Field(
            default=500,
            description="Milliseconds to wait after the action for the UI to settle.",
        )

    async def execute(self, ctx: ToolContext, params: "MouseTool.Params") -> ToolResult:
        from opencua.computer.sandbox import ActionType, get_sandbox

        scaled_params = params
        scale_note = ""
        if params.image_width and params.image_height:
            try:
                pag = _pyautogui()
                screen_w, screen_h = pag.size()
                sx = screen_w / params.image_width
                sy = screen_h / params.image_height
                if abs(sx - 1.0) > 0.02 or abs(sy - 1.0) > 0.02:
                    scaled = params.model_copy(update={
                        "x": round(params.x * sx),
                        "y": round(params.y * sy),
                        "end_x": round(params.end_x * sx) if params.end_x is not None else None,
                        "end_y": round(params.end_y * sy) if params.end_y is not None else None,
                    })
                    scaled_params = scaled
                    scale_note = (
                        f" [image ({params.x},{params.y}) -> logical ({scaled.x},{scaled.y})"
                        f" scale {sx:.3f}x{sy:.3f}]"
                    )
            except Exception:
                pass

        if params.action == "cursor_position":
            await ctx.check_permission(
                tool="mouse", argument="cursor_position",
                description="Query current cursor position",
            )
            try:
                pag = _pyautogui()
                cx, cy = pag.position()
                sandbox = get_sandbox(ctx.session_id)
                await sandbox.record_action(ActionType.CURSOR_POSITION, {}, result=f"({cx},{cy})")
                return ToolResult(title="Cursor position", output=f"Current cursor: ({cx}, {cy})")
            except Exception as exc:
                return ToolResult(title="Cursor position error", output=str(exc), error=True)

        action_label = f"mouse {params.action} at ({params.x}, {params.y})"
        await ctx.check_permission(
            tool="mouse", argument=action_label,
            description=f"Mouse action: {action_label}",
        )

        sandbox = get_sandbox(ctx.session_id)
        if not sandbox.is_coordinate_allowed(scaled_params.x, scaled_params.y):
            return ToolResult(
                title="Mouse action denied",
                output=f"Coordinate ({scaled_params.x}, {scaled_params.y}) is outside the permitted region.",
                error=True,
            )

        action_params = {
            "action": params.action, "x": params.x, "y": params.y,
            "amount": params.amount, "duration": params.duration,
        }
        if params.action == "drag":
            action_params["end_x"] = params.end_x
            action_params["end_y"] = params.end_y

        try:
            loop = asyncio.get_event_loop()
            result_msg = await loop.run_in_executor(None, self._do_action, scaled_params)
        except ImportError as exc:
            return ToolResult(title="Mouse error", output=str(exc), error=True)
        except Exception as exc:
            await sandbox.record_action(ActionType.MOUSE_CLICK, action_params, error=str(exc))
            return ToolResult(title="Mouse error", output=f"Mouse action failed: {exc}", error=True)

        action_type_map = {
            "move": ActionType.MOUSE_MOVE,
            "click": ActionType.MOUSE_CLICK,
            "double_click": ActionType.MOUSE_CLICK,
            "triple_click": ActionType.MOUSE_CLICK,
            "right_click": ActionType.MOUSE_CLICK,
            "middle_click": ActionType.MOUSE_CLICK,
            "left_down": ActionType.MOUSE_DOWN,
            "left_up": ActionType.MOUSE_UP,
            "scroll": ActionType.MOUSE_SCROLL,
            "drag": ActionType.MOUSE_DRAG,
        }
        await sandbox.record_action(
            action_type_map.get(params.action, ActionType.MOUSE_CLICK),
            action_params, result=result_msg,
        )

        return ToolResult(title=f"Mouse: {params.action}", output=result_msg + scale_note)

    @staticmethod
    def _do_action(params: "MouseTool.Params") -> str:
        import time
        _check_accessibility()
        pag = _pyautogui()
        pag.FAILSAFE = True
        settle = params.settle_ms / 1000.0

        if params.action == "move":
            pag.moveTo(params.x, params.y, duration=params.duration)
            time.sleep(settle)
            return f"Moved mouse to ({params.x}, {params.y})."

        if params.action == "click":
            pag.click(params.x, params.y, duration=params.duration)
            time.sleep(settle)
            return f"Left-clicked at ({params.x}, {params.y})."

        if params.action == "double_click":
            pag.doubleClick(params.x, params.y, duration=params.duration)
            time.sleep(settle)
            return f"Double-clicked at ({params.x}, {params.y})."

        if params.action == "triple_click":
            pag.click(params.x, params.y, duration=params.duration, clicks=3, interval=0.05)
            time.sleep(settle)
            return f"Triple-clicked at ({params.x}, {params.y})."

        if params.action == "right_click":
            pag.rightClick(params.x, params.y, duration=params.duration)
            time.sleep(settle)
            return f"Right-clicked at ({params.x}, {params.y})."

        if params.action == "middle_click":
            pag.middleClick(params.x, params.y)
            time.sleep(settle)
            return f"Middle-clicked at ({params.x}, {params.y})."

        if params.action == "left_down":
            pag.moveTo(params.x, params.y, duration=params.duration)
            pag.mouseDown(button="left")
            time.sleep(settle)
            return f"Left button pressed at ({params.x}, {params.y}) -- button is held."

        if params.action == "left_up":
            pag.moveTo(params.x, params.y, duration=params.duration)
            pag.mouseUp(button="left")
            time.sleep(settle)
            return f"Left button released at ({params.x}, {params.y})."

        if params.action == "scroll":
            direction = params.direction
            if direction in ("up", "down"):
                clicks = params.amount if direction == "up" else -params.amount
                pag.scroll(clicks, x=params.x, y=params.y)
                time.sleep(settle)
                return f"Scrolled {direction} {params.amount} click(s) at ({params.x}, {params.y})."
            if direction in ("left", "right"):
                clicks = params.amount if direction == "right" else -params.amount
                pag.hscroll(clicks, x=params.x, y=params.y)
                time.sleep(settle)
                return f"Scrolled {direction} {params.amount} click(s) at ({params.x}, {params.y})."
            pag.scroll(params.amount, x=params.x, y=params.y)
            time.sleep(settle)
            scroll_dir = "up" if params.amount > 0 else "down"
            return f"Scrolled {scroll_dir} by {abs(params.amount)} click(s) at ({params.x}, {params.y})."

        if params.action == "drag":
            if params.end_x is None or params.end_y is None:
                raise ValueError("end_x and end_y are required for drag action.")
            pag.moveTo(params.x, params.y, duration=0.1)
            pag.dragTo(params.end_x, params.end_y, duration=params.duration, mouseDownUp=True)
            time.sleep(params.settle_ms / 1000.0)
            return f"Dragged from ({params.x}, {params.y}) to ({params.end_x}, {params.end_y})."

        raise ValueError(f"Unknown mouse action: {params.action!r}")
