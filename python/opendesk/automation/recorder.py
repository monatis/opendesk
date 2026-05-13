"""Global pynput recorder with async screenshot capture."""

from __future__ import annotations

import base64
import io
import threading
import time
from concurrent.futures import Future, ThreadPoolExecutor
from typing import Optional

from opendesk.automation.trajectory import Trajectory, TrajectoryEvent

_KEY_FLUSH_DELAY = 0.55
_SS_MAX_W = 800
_SS_MAX_H = 500
_SS_QUALITY = 55

_SPECIAL_KEY_NAMES: dict = {}

_MODIFIER_NAMES = {
    "cmd", "cmd_l", "cmd_r",
    "ctrl", "ctrl_l", "ctrl_r",
    "alt", "alt_l", "alt_r",
    "shift", "shift_l", "shift_r",
}


class LearnRecorder:
    """Record global mouse + keyboard events with async screenshot capture."""

    def __init__(self, task_name: str) -> None:
        self._task_name = task_name
        self._trajectory = Trajectory(task_name=task_name)
        self._lock = threading.Lock()
        self._executor = ThreadPoolExecutor(max_workers=2, thread_name_prefix="learn-ss")
        self._pending: list = []
        self._key_buffer: list = []
        self._flush_timer: Optional[threading.Timer] = None
        self._held_mods: set = set()
        self._mouse_listener = None
        self._keyboard_listener = None
        self._active = False

    def start(self) -> None:
        try:
            from pynput import keyboard, mouse
            _init_special_keys(keyboard)
        except ImportError:
            raise RuntimeError(
                "pynput is required for learn recording.\n"
                "  pip install pynput"
            )

        self._active = True
        self._submit_screenshot(attach_to=None, is_initial=True)

        from pynput import keyboard as kb, mouse as ms

        self._mouse_listener = ms.Listener(
            on_click=self._on_click,
            on_scroll=self._on_scroll,
        )
        self._keyboard_listener = kb.Listener(
            on_press=self._on_key_press,
            on_release=self._on_key_release,
        )
        self._mouse_listener.start()
        self._keyboard_listener.start()

    def stop(self) -> Trajectory:
        self._active = False
        self._flush_key_buffer()

        if self._mouse_listener is not None:
            self._mouse_listener.stop()
        if self._keyboard_listener is not None:
            self._keyboard_listener.stop()

        self._executor.shutdown(wait=False)
        self._trajectory.stopped_at = time.time()
        return self._trajectory

    def _on_click(self, x: int, y: int, button, pressed: bool) -> None:
        if not pressed or not self._active:
            return
        self._flush_key_buffer()

        try:
            from pynput.mouse import Button
            action = "right_click" if button == Button.right else "click"
        except Exception:
            action = "click"

        event = TrajectoryEvent(
            timestamp=time.time(),
            action_type=action,
            x=int(x),
            y=int(y),
            button=str(button).split(".")[-1],
        )
        with self._lock:
            self._trajectory.events.append(event)
        self._submit_screenshot(attach_to=event)

    def _on_scroll(self, x: int, y: int, dx: int, dy: int) -> None:
        if not self._active:
            return
        with self._lock:
            if (
                self._trajectory.events
                and self._trajectory.events[-1].action_type == "scroll"
                and self._trajectory.events[-1].x == int(x)
                and self._trajectory.events[-1].y == int(y)
            ):
                ev = self._trajectory.events[-1]
                ev.scroll_dx += dx
                ev.scroll_dy += dy
                return
            event = TrajectoryEvent(
                timestamp=time.time(),
                action_type="scroll",
                x=int(x),
                y=int(y),
                scroll_dx=dx,
                scroll_dy=dy,
            )
            self._trajectory.events.append(event)

    def _on_key_press(self, key) -> None:
        if not self._active:
            return
        try:
            key_name = _key_name(key)

            if key_name in _MODIFIER_NAMES:
                with self._lock:
                    self._held_mods.add(_normalise_mod(key_name))
                return

            with self._lock:
                mods = frozenset(self._held_mods)

            if mods:
                combo = "+".join(sorted(mods) + [key_name])
                self._flush_key_buffer()
                event = TrajectoryEvent(
                    timestamp=time.time(),
                    action_type="key",
                    key=combo,
                )
                with self._lock:
                    self._trajectory.events.append(event)
                return

            if key_name in _SPECIAL_KEY_NAMES:
                self._flush_key_buffer()
                event = TrajectoryEvent(
                    timestamp=time.time(),
                    action_type="key",
                    key=_SPECIAL_KEY_NAMES[key_name],
                )
                with self._lock:
                    self._trajectory.events.append(event)
                if _SPECIAL_KEY_NAMES[key_name] in ("return", "escape"):
                    self._submit_screenshot(attach_to=event)
                return

            char = getattr(key, "char", None)
            if char and char.isprintable():
                self._cancel_flush_timer()
                with self._lock:
                    self._key_buffer.append(char)
                self._schedule_flush()

        except Exception:
            pass

    def _on_key_release(self, key) -> None:
        if not self._active:
            return
        try:
            key_name = _normalise_mod(_key_name(key))
            with self._lock:
                self._held_mods.discard(key_name)
        except Exception:
            pass

    def _schedule_flush(self) -> None:
        self._cancel_flush_timer()
        t = threading.Timer(_KEY_FLUSH_DELAY, self._flush_key_buffer)
        t.daemon = True
        self._flush_timer = t
        t.start()

    def _cancel_flush_timer(self) -> None:
        if self._flush_timer is not None:
            self._flush_timer.cancel()
            self._flush_timer = None

    def _flush_key_buffer(self) -> None:
        self._cancel_flush_timer()
        with self._lock:
            if not self._key_buffer:
                return
            text = "".join(self._key_buffer)
            self._key_buffer.clear()
        event = TrajectoryEvent(
            timestamp=time.time(),
            action_type="type",
            text=text,
        )
        with self._lock:
            self._trajectory.events.append(event)
        self._submit_accessibility(attach_to=event)

    def _submit_screenshot(self, attach_to: Optional[TrajectoryEvent], is_initial: bool = False) -> None:
        future = self._executor.submit(self._capture, attach_to, is_initial)
        with self._lock:
            self._pending = [f for f in self._pending if not f.done()]
            self._pending.append(future)

    def _submit_accessibility(self, attach_to: TrajectoryEvent) -> None:
        def _do() -> None:
            attach_to.accessibility_context = _ax_focused_context()
        future = self._executor.submit(_do)
        with self._lock:
            self._pending = [f for f in self._pending if not f.done()]
            self._pending.append(future)

    def _capture(self, attach_to: Optional[TrajectoryEvent], is_initial: bool) -> None:
        if attach_to is not None and not is_initial:
            x = attach_to.x
            y = attach_to.y
            if x is not None and y is not None:
                attach_to.accessibility_context = _ax_context_at(float(x), float(y))
            else:
                attach_to.accessibility_context = _ax_focused_context()

        try:
            import mss
            from PIL import Image

            with mss.mss() as sct:
                raw = sct.grab(sct.monitors[0])
                img = Image.frombytes("RGB", raw.size, raw.rgb)

            img.thumbnail((_SS_MAX_W, _SS_MAX_H), Image.LANCZOS)

            buf = io.BytesIO()
            img.save(buf, format="JPEG", quality=_SS_QUALITY, optimize=True)
            b64 = base64.b64encode(buf.getvalue()).decode("ascii")

            if is_initial:
                self._trajectory.initial_screenshot = b64
            elif attach_to is not None:
                attach_to.screenshot_after = b64

        except Exception:
            pass


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _init_special_keys(keyboard_module) -> None:
    global _SPECIAL_KEY_NAMES
    if _SPECIAL_KEY_NAMES:
        return
    _SPECIAL_KEY_NAMES = {
        "enter": "return", "return": "return",
        "esc": "escape", "escape": "escape",
        "tab": "tab", "backspace": "backspace",
        "delete": "delete", "up": "up", "down": "down",
        "left": "left", "right": "right",
        "home": "home", "end": "end",
        "page_up": "page_up", "page_down": "page_down",
        "space": "space",
        "f1": "f1", "f2": "f2", "f3": "f3", "f4": "f4",
        "f5": "f5", "f6": "f6", "f7": "f7", "f8": "f8",
        "f9": "f9", "f10": "f10", "f11": "f11", "f12": "f12",
    }


def _key_name(key) -> str:
    name = str(key)
    if name.startswith("Key."):
        return name[4:].lower()
    if hasattr(key, "vk") and key.vk and not hasattr(key, "char"):
        return f"vk_{key.vk}"
    if hasattr(key, "char") and key.char:
        return key.char.lower()
    return name.lower().strip("'")


def _normalise_mod(name: str) -> str:
    return name.split("_")[0] if "_" in name and name.split("_")[1] in ("l", "r") else name


def _ax_attr(obj: object, *attrs: str) -> str:
    for attr in attrs:
        try:
            val = getattr(obj, attr, None)
            if val and isinstance(val, str):
                return val.strip()
        except Exception:
            pass
    return ""


def _ax_context_at(x: float, y: float) -> dict:
    ctx: dict = {}
    try:
        import atomacos
        app = atomacos.getFrontmostApp()
        ctx["app"] = _ax_attr(app, "AXTitle")
        try:
            win = app.AXFocusedWindow
            ctx["window"] = _ax_attr(win, "AXTitle")
        except Exception:
            pass
        try:
            system = atomacos.NativeUIElement.systemwide()
            elem = system.getElementAtPosition(x, y)
            ctx["role"] = _ax_attr(elem, "AXRole", "AXRoleDescription")
            ctx["title"] = _ax_attr(elem, "AXTitle", "AXDescription", "AXLabel")
            raw_val = getattr(elem, "AXValue", None)
            if raw_val is not None:
                ctx["value"] = str(raw_val)[:80]
        except Exception:
            try:
                elem = app.AXFocusedUIElement
                ctx["role"] = _ax_attr(elem, "AXRole", "AXRoleDescription")
                ctx["title"] = _ax_attr(elem, "AXTitle", "AXDescription", "AXLabel")
            except Exception:
                pass
    except ImportError:
        pass
    except Exception:
        pass
    return {k: v for k, v in ctx.items() if v}


def _ax_focused_context() -> dict:
    ctx: dict = {}
    try:
        import atomacos
        app = atomacos.getFrontmostApp()
        ctx["app"] = _ax_attr(app, "AXTitle")
        try:
            win = app.AXFocusedWindow
            ctx["window"] = _ax_attr(win, "AXTitle")
        except Exception:
            pass
        try:
            elem = app.AXFocusedUIElement
            ctx["role"] = _ax_attr(elem, "AXRole", "AXRoleDescription")
            ctx["title"] = _ax_attr(elem, "AXTitle", "AXDescription", "AXLabel")
        except Exception:
            pass
    except ImportError:
        pass
    except Exception:
        pass
    return {k: v for k, v in ctx.items() if v}
