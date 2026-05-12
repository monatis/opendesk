"""UITool — accessibility-tree based UI interaction, no pixel coordinates needed.

This is the PRIMARY interaction tool for any element that has a visible label or
title.  It clicks buttons, types text, navigates menus, and reads element values
through the platform's native accessibility API.

Why this beats coordinate-based mouse clicks
--------------------------------------------
* No Retina / HiDPI scaling translation needed.
* Works at any window position or screen resolution.
* Errors are descriptive: "button 'Save' not found" vs a silent mis-click.
* Uses the same accessibility layer as screen readers.

Platform backends
-----------------
macOS   — AppleScript / System Events (built-in, no extra deps)
Linux   — AT-SPI2 via pyatspi (preferred) with xdotool fallbacks
Windows — UI Automation via pywinauto (preferred) with Win32 fallbacks
"""

from __future__ import annotations

import platform
import subprocess
import time
from typing import Any, List, Literal, Optional

from pydantic import Field

from opendesk.tools.base import Tool, ToolContext, ToolResult

_PLATFORM = platform.system()


class UITool(Tool):
    """Interact with desktop UI elements by name — no pixel coordinates needed.

    ALWAYS try this before the mouse tool.  Use mouse clicks only for
    unlabelled canvas areas (games, video players, drawing tools) where no
    accessible elements exist.
    """

    name = "ui"
    description = (
        "Interact with UI elements by name on any OS — no coordinates needed. "
        "ALWAYS try this before the mouse tool. Actions:\n"
        "  get_tree   — list all accessible elements in the app window\n"
        "  click      — click a button or element by its title\n"
        "  click_menu — click a menu item, e.g. File → Save\n"
        "  type       — type text (clipboard-paste, Unicode-safe)\n"
        "  press_key  — press a key or chord: key='return', modifiers=['command']\n"
        "  get_value  — read the current text value of a named element"
    )

    class Params(Tool.Params):
        action: Literal["get_tree", "click", "click_menu", "type", "press_key", "get_value"] = Field(
            description="UI action to perform."
        )
        app: str = Field(
            description=(
                "Application name / process name. "
                "macOS: process name as in Activity Monitor, e.g. 'TextEdit', 'Safari'. "
                "Linux: process or window title, e.g. 'gedit', 'firefox'. "
                "Windows: executable name or title, e.g. 'Notepad', 'notepad.exe'."
            )
        )
        title: Optional[str] = Field(default=None, description="Title or label of the UI element.")
        role: Optional[str] = Field(
            default=None,
            description=(
                "Element role to narrow the search. "
                "macOS: 'button', 'text field', 'text area', 'checkbox'. "
                "Linux: 'push button', 'entry', 'text', 'check box'. "
                "Windows: 'Button', 'Edit', 'Text', 'CheckBox', 'ComboBox'."
            ),
        )
        text: Optional[str] = Field(default=None, description="Text to type. Required for action='type'.")
        menu: Optional[str] = Field(
            default=None,
            description="Menu bar menu name for click_menu, e.g. 'File', 'Edit'.",
        )
        menu_item: Optional[str] = Field(
            default=None,
            description="Menu item name for click_menu, e.g. 'Save', 'Copy'.",
        )
        key: Optional[str] = Field(
            default=None,
            description=(
                "Key for press_key: 'return', 'escape', 'tab', 'space', 'delete', "
                "'up', 'down', 'left', 'right', 'home', 'end', 'f1'–'f12', or a single char."
            ),
        )
        modifiers: List[str] = Field(
            default_factory=list,
            description=(
                "Modifier keys for press_key: 'command' (macOS), 'ctrl', 'shift', 'alt'. "
                "E.g. ['command'] for cmd+key."
            ),
        )
        window_index: int = Field(default=1, description="Window index (1 = frontmost).")

    async def execute(self, ctx: ToolContext, params: "UITool.Params") -> ToolResult:
        import asyncio
        from opendesk.computer.sandbox import ActionType, get_sandbox

        await ctx.check_permission(
            tool="ui", argument=f"{params.action} in {params.app}",
            description=f"UI accessibility action: {params.action} in '{params.app}'",
        )

        sandbox = get_sandbox(ctx.session_id)
        loop = asyncio.get_event_loop()

        try:
            result_msg = await loop.run_in_executor(None, self._do_action, params)
        except (RuntimeError, ValueError, ImportError) as exc:
            await sandbox.record_action(
                ActionType.UI_ACTION,
                {"action": params.action, "app": params.app, "title": params.title},
                error=str(exc),
            )
            return ToolResult(
                title=f"UI error: {params.action} in {params.app}",
                output=str(exc),
                error=True,
            )
        except Exception as exc:
            return ToolResult(title=f"UI error: {params.action}", output=str(exc), error=True)

        await sandbox.record_action(
            ActionType.UI_ACTION,
            {"action": params.action, "app": params.app, "title": params.title},
            result=result_msg[:200],
        )
        label = params.title or params.menu_item or params.key or ""
        return ToolResult(
            title=f"UI: {params.action} '{label}' in {params.app}",
            output=result_msg,
        )

    @staticmethod
    def _do_action(params: "UITool.Params") -> str:
        if _PLATFORM == "Darwin":
            return _macos_dispatch(params)
        if _PLATFORM == "Linux":
            return _linux_dispatch(params)
        if _PLATFORM == "Windows":
            return _windows_dispatch(params)
        raise RuntimeError(
            f"UITool: unsupported platform '{_PLATFORM}'. "
            "Use the mouse tool with image_width/image_height instead."
        )


# ===========================================================================
# macOS — AppleScript / System Events
# ===========================================================================

def _osascript(script: str, timeout: int = 15) -> str:
    r = subprocess.run(["osascript", "-e", script], capture_output=True, text=True, timeout=timeout)
    if r.returncode != 0:
        raise RuntimeError((r.stderr or r.stdout).strip())
    return r.stdout.strip()


def _macos_dispatch(params: "UITool.Params") -> str:
    if params.action == "get_tree":
        return _macos_get_tree(params.app, params.window_index)
    if params.action == "click":
        return _macos_click(params.app, params.title, params.role, params.window_index)
    if params.action == "click_menu":
        return _macos_click_menu(params.app, params.menu, params.menu_item)
    if params.action == "type":
        return _macos_type(params.app, params.text)
    if params.action == "press_key":
        return _macos_press_key(params.app, params.key, params.modifiers)
    if params.action == "get_value":
        return _macos_get_value(params.app, params.title, params.role, params.window_index)
    raise ValueError(f"Unknown action: {params.action!r}")


def _macos_get_tree(app: str, window_index: int = 1) -> str:
    script = f"""
tell application "System Events"
    tell process "{app}"
        set win to window {window_index}
        set winTitle to ""
        try
            set winTitle to title of win
        end try
        set output to "Window " & {window_index} & ": \\"" & winTitle & "\\"\\n"
        set allElems to entire contents of win
        repeat with elem in allElems
            try
                set r to role of elem
                set t to ""
                try
                    set t to title of elem
                end try
                if t is "" then
                    try
                        set t to value of elem as text
                    on error
                        set t to ""
                    end try
                end if
                if t is "" then
                    try
                        set t to description of elem
                    end try
                end if
                if t is not "" and t is not missing value then
                    set output to output & "  [" & r & "] " & t & "\\n"
                end if
            end try
        end repeat
        return output
    end tell
end tell
"""
    result = _osascript(script)
    if not result.strip():
        return (
            f"No accessible elements found in {app} window {window_index}. "
            "App may use custom rendering (Electron, game engines). "
            "Fall back to mouse tool with image_width/image_height."
        )
    return result


def _macos_click(app: str, title: str | None, role: str | None, window_index: int = 1) -> str:
    if not title and not role:
        raise ValueError("Provide at least 'title' or 'role' for click.")
    if title:
        escaped = title.replace('"', '\\"')
        script = f"""
tell application "System Events"
    tell process "{app}"
        set allElems to entire contents of window {window_index}
        repeat with elem in allElems
            try
                set matched to false
                try
                    if title of elem is "{escaped}" then set matched to true
                end try
                if not matched then
                    try
                        if description of elem is "{escaped}" then set matched to true
                    end try
                end if
                if matched then
                    click elem
                    return "Clicked [" & (role of elem) & "] \\"{escaped}\\" in {app}."
                end if
            end try
        end repeat
        error "No element titled \\"{escaped}\\" in {app} window {window_index}. Run get_tree to list elements."
    end tell
end tell
"""
    else:
        script = f"""
tell application "System Events"
    tell process "{app}"
        click first {role} of window {window_index}
        return "Clicked first [{role}] in {app}."
    end tell
end tell
"""
    return _osascript(script)


def _macos_click_menu(app: str, menu: str | None, menu_item: str | None) -> str:
    if not menu or not menu_item:
        raise ValueError("Both 'menu' and 'menu_item' are required for click_menu.")
    em = menu.replace('"', '\\"')
    ei = menu_item.replace('"', '\\"')
    script = f"""
tell application "System Events"
    tell process "{app}"
        click menu item "{ei}" of menu "{em}" of menu bar item "{em}" of menu bar 1
        return "Clicked {menu} → {menu_item} in {app}."
    end tell
end tell
"""
    return _osascript(script)


def _macos_type(app: str, text: str | None) -> str:
    if not text:
        raise ValueError("'text' is required for action='type'.")
    _osascript(f'tell application "{app}" to activate')
    time.sleep(0.3)
    subprocess.run(["pbcopy"], input=text.encode("utf-8"), check=True)
    time.sleep(0.1)
    _osascript('tell application "System Events" to keystroke "v" using {command down}')
    time.sleep(0.2)
    preview = text[:60] + ("…" if len(text) > 60 else "")
    return f"Typed {len(text)} chars into {app}: {preview!r}"


_MACOS_KEY_MAP: dict[str, tuple[str, bool]] = {
    "return": ("return", False), "enter": ("return", False),
    "escape": ("escape", False), "esc": ("escape", False),
    "tab": ("tab", False), "space": ("space", False),
    "delete": ("delete", False), "backspace": ("delete", False),
    "up": ("126", True), "down": ("125", True),
    "left": ("123", True), "right": ("124", True),
    "home": ("115", True), "end": ("119", True),
    "pageup": ("116", True), "pagedown": ("121", True),
    **{f"f{i}": (str(v), True) for i, v in zip(range(1, 13),
       [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111])},
}
_MACOS_MOD_MAP = {
    "command": "command down", "cmd": "command down",
    "shift": "shift down", "option": "option down",
    "alt": "option down", "control": "control down", "ctrl": "control down",
}


def _macos_press_key(app: str, key: str | None, modifiers: list[str]) -> str:
    if not key:
        raise ValueError("'key' is required for action='press_key'.")
    _osascript(f'tell application "{app}" to activate')
    time.sleep(0.15)
    mod_parts = [_MACOS_MOD_MAP.get(m.lower(), f"{m} down") for m in modifiers]
    mod_str = "{" + ", ".join(mod_parts) + "}" if mod_parts else ""
    kl = key.lower()
    if kl in _MACOS_KEY_MAP:
        val, is_code = _MACOS_KEY_MAP[kl]
        verb = "key code" if is_code else "keystroke"
        script = (
            f'tell application "System Events" to {verb} {val} using {mod_str}'
            if mod_str else
            f'tell application "System Events" to {verb} {val}'
        )
    else:
        esc = key.replace('"', '\\"')
        script = (
            f'tell application "System Events" to keystroke "{esc}" using {mod_str}'
            if mod_str else
            f'tell application "System Events" to keystroke "{esc}"'
        )
    _osascript(script)
    combo = ("+".join(modifiers) + "+" if modifiers else "") + key
    return f"Pressed {combo} in {app}."


def _macos_get_value(app: str, title: str | None, role: str | None, window_index: int = 1) -> str:
    if title:
        esc = title.replace('"', '\\"')
        script = f"""
tell application "System Events"
    tell process "{app}"
        set allElems to entire contents of window {window_index}
        repeat with elem in allElems
            try
                if title of elem is "{esc}" then
                    return value of elem as text
                end if
            end try
        end repeat
        error "No element titled \\"{esc}\\" found."
    end tell
end tell
"""
    elif role:
        script = f"""
tell application "System Events"
    tell process "{app}"
        return value of first {role} of window {window_index} as text
    end tell
end tell
"""
    else:
        raise ValueError("Provide 'title' or 'role' for get_value.")
    return _osascript(script)


# ===========================================================================
# Linux — AT-SPI2 (pyatspi) preferred, xdotool fallback
# ===========================================================================

def _xdotool(*args: str, check: bool = False) -> subprocess.CompletedProcess:  # type: ignore[type-arg]
    return subprocess.run(["xdotool", *args], capture_output=True, text=True, timeout=10, check=check)


def _has_xdotool() -> bool:
    return subprocess.run(["which", "xdotool"], capture_output=True).returncode == 0


def _linux_dispatch(params: "UITool.Params") -> str:
    try:
        import pyatspi  # type: ignore[import-not-found]
        return _linux_atspi_dispatch(params)
    except ImportError:
        return _linux_xdotool_dispatch(params)


def _atspi_find_app(name: str) -> Any:
    import pyatspi
    desktop = pyatspi.Registry.getDesktop(0)
    name_lower = name.lower()
    for app in desktop:
        if app and app.name and app.name.lower() == name_lower:
            return app
    for app in desktop:
        if app and app.name and name_lower in app.name.lower():
            return app
    available = [a.name for a in desktop if a and a.name]
    raise RuntimeError(
        f"App '{name}' not found in AT-SPI tree. "
        f"Running apps: {', '.join(available) or 'none'}."
    )


def _atspi_walk(node: Any, depth: int = 0, max_depth: int = 10):
    if depth > max_depth:
        return
    yield node
    try:
        for i in range(node.childCount):
            try:
                yield from _atspi_walk(node.getChildAtIndex(i), depth + 1, max_depth)
            except Exception:
                pass
    except Exception:
        pass


def _atspi_find(app_node: Any, title: str | None, role: str | None) -> Any:
    title_lower = title.lower() if title else None
    role_lower = role.lower() if role else None
    for node in _atspi_walk(app_node):
        try:
            name = (node.name or "").lower()
            node_role = (node.getLocalizedRoleName() or "").lower()
            name_ok = not title_lower or title_lower in name
            role_ok = not role_lower or role_lower in node_role
            if name_ok and role_ok and (title_lower or role_lower):
                if node.name or node.getLocalizedRoleName():
                    return node
        except Exception:
            pass
    return None


def _linux_atspi_dispatch(params: "UITool.Params") -> str:
    if params.action == "get_tree":
        app = _atspi_find_app(params.app)
        lines: list[str] = [f"App: {app.name}"]
        seen = 0
        for node in _atspi_walk(app, max_depth=6):
            if seen >= 150:
                lines.append("  … (truncated)")
                break
            try:
                name = node.name or ""
                role = node.getLocalizedRoleName() or ""
                if name or role:
                    lines.append(f"  [{role}] {name}")
                    seen += 1
            except Exception:
                pass
        return "\n".join(lines) if len(lines) > 1 else (
            f"No accessible elements in '{params.app}'. Fall back to mouse tool."
        )

    if params.action == "click":
        import pyatspi
        app = _atspi_find_app(params.app)
        node = _atspi_find(app, params.title, params.role)
        if node is None:
            raise RuntimeError(
                f"No element title='{params.title}' role='{params.role}' in '{params.app}'. "
                "Run get_tree to see available elements."
            )
        try:
            action = node.queryAction()
            action_names = [action.getName(i).lower() for i in range(action.nActions)]
            for pref in ("click", "press", "activate", "toggle"):
                if pref in action_names:
                    action.doAction(action_names.index(pref))
                    return f"Clicked [{node.getLocalizedRoleName()}] '{node.name}' in {params.app}."
            if action.nActions > 0:
                action.doAction(0)
                return f"Activated [{node.getLocalizedRoleName()}] '{node.name}' in {params.app}."
        except Exception:
            pass
        try:
            bbox = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
            cx, cy = bbox.x + bbox.width // 2, bbox.y + bbox.height // 2
            _xdotool("mousemove", "--sync", str(cx), str(cy))
            _xdotool("click", "1")
            return f"Clicked at ({cx},{cy}) '{node.name}' in {params.app}."
        except Exception as exc:
            raise RuntimeError(f"Could not click element '{params.title}': {exc}") from exc

    if params.action == "type":
        return _linux_type(params.app, params.text)

    if params.action == "press_key":
        return _linux_press_key(params.app, params.key, params.modifiers)

    if params.action == "get_value":
        app = _atspi_find_app(params.app)
        node = _atspi_find(app, params.title, params.role)
        if node is None:
            raise RuntimeError(f"No element title='{params.title}' role='{params.role}' in '{params.app}'.")
        try:
            val = node.queryValue()
            return str(val.currentValue)
        except Exception:
            pass
        try:
            text = node.queryText()
            return text.getText(0, -1)
        except Exception:
            pass
        return node.name or "(no value)"

    raise ValueError(f"Unknown action: {params.action!r}")


def _linux_xdotool_dispatch(params: "UITool.Params") -> str:
    if params.action == "type":
        return _linux_type(params.app, params.text)
    if params.action == "press_key":
        return _linux_press_key(params.app, params.key, params.modifiers)
    raise ImportError(
        f"action='{params.action}' requires pyatspi: pip install pyatspi "
        "(or: sudo apt install python3-pyatspi)"
    )


def _linux_focus_window(app_name: str) -> None:
    r = _xdotool("search", "--name", app_name, "windowactivate", "--sync")
    if r.returncode != 0:
        _xdotool("search", "--class", app_name, "windowactivate", "--sync")
    time.sleep(0.2)


def _linux_type(app_name: str, text: str | None) -> str:
    if not text:
        raise ValueError("'text' is required for action='type'.")
    _linux_focus_window(app_name)
    for copy_cmd, paste_cmd in [
        (["xclip", "-selection", "clipboard"], ["xdotool", "key", "--clearmodifiers", "ctrl+v"]),
        (["xsel", "--clipboard", "--input"],    ["xdotool", "key", "--clearmodifiers", "ctrl+v"]),
    ]:
        r = subprocess.run(copy_cmd, input=text.encode("utf-8"), capture_output=True, timeout=5)
        if r.returncode == 0:
            time.sleep(0.1)
            subprocess.run(paste_cmd, check=True, timeout=5)
            preview = text[:60] + ("…" if len(text) > 60 else "")
            return f"Typed {len(text)} chars into {app_name}: {preview!r}"
    if _has_xdotool():
        _xdotool("type", "--clearmodifiers", "--delay", "20", text, check=True)
        return f"Typed {len(text)} chars into {app_name}."
    raise RuntimeError("Could not type: install xclip: sudo apt install xclip")


_LINUX_KEY_MAP = {
    "return": "Return", "enter": "Return", "escape": "Escape", "esc": "Escape",
    "tab": "Tab", "space": "space", "delete": "Delete", "backspace": "BackSpace",
    "up": "Up", "down": "Down", "left": "Left", "right": "Right",
    "home": "Home", "end": "End", "pageup": "Prior", "pagedown": "Next",
    **{f"f{i}": f"F{i}" for i in range(1, 13)},
}
_LINUX_MOD_MAP = {
    "command": "ctrl", "cmd": "ctrl", "ctrl": "ctrl", "control": "ctrl",
    "shift": "shift", "alt": "alt", "option": "alt",
}


def _linux_press_key(app_name: str, key: str | None, modifiers: list[str]) -> str:
    if not key:
        raise ValueError("'key' is required for action='press_key'.")
    if not _has_xdotool():
        raise RuntimeError("xdotool required: sudo apt install xdotool")
    _linux_focus_window(app_name)
    mapped_key = _LINUX_KEY_MAP.get(key.lower(), key)
    mapped_mods = [_LINUX_MOD_MAP.get(m.lower(), m) for m in modifiers]
    combo = "+".join(mapped_mods + [mapped_key]) if mapped_mods else mapped_key
    _xdotool("key", "--clearmodifiers", combo, check=True)
    display = ("+".join(modifiers) + "+" if modifiers else "") + key
    return f"Pressed {display} in {app_name}."


# ===========================================================================
# Windows — pywinauto (UI Automation) preferred
# ===========================================================================

_WIN_ROLE_MAP = {
    "button": "Button", "btn": "Button",
    "edit": "Edit", "text field": "Edit", "input": "Edit",
    "text": "Text", "label": "Text", "static": "Text",
    "checkbox": "CheckBox", "check box": "CheckBox",
    "combobox": "ComboBox", "combo box": "ComboBox", "dropdown": "ComboBox",
    "listitem": "ListItem", "list item": "ListItem",
    "tab": "TabItem", "tab item": "TabItem",
    "menu item": "MenuItem", "menuitem": "MenuItem",
}


def _win_connect(app_name: str) -> Any:
    from opendesk.computer.deps import ensure_import
    _pywinauto = ensure_import("pywinauto")
    Application = _pywinauto.Application  # type: ignore[attr-defined]
    errors: list[str] = []
    for backend in ("uia", "win32"):
        for kwargs in ({"path": app_name}, {"title_re": f".*{app_name}.*"}, {"title": app_name}):
            try:
                return Application(backend=backend).connect(**kwargs, timeout=3)
            except Exception as e:
                errors.append(str(e))
    raise RuntimeError(f"Could not connect to '{app_name}'. Is it running? Tried: {'; '.join(errors[:3])}")


def _windows_dispatch(params: "UITool.Params") -> str:
    if params.action == "get_tree":
        app = _win_connect(params.app)
        dlg = app.top_window()
        lines = [f"Window: {dlg.window_text()}"]
        seen = 0
        try:
            for ctrl in dlg.descendants():
                if seen >= 150:
                    lines.append("  … (truncated)")
                    break
                try:
                    title = ctrl.window_text().strip()
                    role = ctrl.friendly_class_name()
                    if title:
                        lines.append(f"  [{role}] {title}")
                        seen += 1
                except Exception:
                    pass
        except Exception:
            pass
        return "\n".join(lines) if len(lines) > 1 else (
            f"No accessible elements in '{params.app}'. Fall back to mouse tool."
        )

    if params.action == "click":
        app = _win_connect(params.app)
        dlg = app.top_window()
        win_role = _WIN_ROLE_MAP.get((params.role or "").lower())
        try:
            if params.title and win_role:
                ctrl = dlg.child_window(title=params.title, control_type=win_role)
            elif params.title:
                ctrl = dlg.child_window(title=params.title)
            elif win_role:
                ctrl = dlg.child_window(control_type=win_role)
            else:
                raise ValueError("Provide 'title' or 'role' for click.")
            ctrl.click_input()
            return f"Clicked '{params.title or params.role}' in {params.app}."
        except Exception as exc:
            raise RuntimeError(
                f"Could not click '{params.title or params.role}' in '{params.app}'. "
                f"Run get_tree. Error: {exc}"
            ) from exc

    if params.action == "click_menu":
        if not params.menu or not params.menu_item:
            raise ValueError("Both 'menu' and 'menu_item' are required.")
        app = _win_connect(params.app)
        dlg = app.top_window()
        try:
            dlg.menu_select(f"{params.menu}->{params.menu_item}")
            return f"Clicked {params.menu} → {params.menu_item} in {params.app}."
        except Exception as exc:
            raise RuntimeError(f"Could not click menu '{params.menu}→{params.menu_item}': {exc}") from exc

    if params.action == "type":
        if not params.text:
            raise ValueError("'text' is required for action='type'.")
        from opendesk.computer.deps import ensure_import
        pyperclip = ensure_import("pyperclip")
        pyperclip.copy(params.text)
        app = _win_connect(params.app)
        app.top_window().type_keys("^v")
        time.sleep(0.15)
        preview = params.text[:60] + ("…" if len(params.text) > 60 else "")
        return f"Typed {len(params.text)} chars into {params.app}: {preview!r}"

    if params.action == "press_key":
        if not params.key:
            raise ValueError("'key' is required.")
        _WIN_KEY_MAP = {
            "return": "{ENTER}", "enter": "{ENTER}", "escape": "{ESC}", "esc": "{ESC}",
            "tab": "{TAB}", "space": " ", "delete": "{DELETE}", "backspace": "{BACKSPACE}",
            "up": "{UP}", "down": "{DOWN}", "left": "{LEFT}", "right": "{RIGHT}",
            "home": "{HOME}", "end": "{END}", "pageup": "{PGUP}", "pagedown": "{PGDN}",
            **{f"f{i}": f"{{F{i}}}" for i in range(1, 13)},
        }
        _WIN_MOD_PREFIX = {
            "command": "^", "cmd": "^", "ctrl": "^", "control": "^",
            "shift": "+", "alt": "%", "option": "%",
        }
        app = _win_connect(params.app)
        prefix = "".join(_WIN_MOD_PREFIX.get(m.lower(), "") for m in params.modifiers)
        key_str = _WIN_KEY_MAP.get(params.key.lower(), params.key if len(params.key) == 1 else f"{{{params.key.upper()}}}")
        app.top_window().type_keys(f"{prefix}{key_str}")
        display = ("+".join(params.modifiers) + "+" if params.modifiers else "") + params.key
        return f"Pressed {display} in {params.app}."

    if params.action == "get_value":
        app = _win_connect(params.app)
        dlg = app.top_window()
        win_role = _WIN_ROLE_MAP.get((params.role or "").lower())
        try:
            if params.title and win_role:
                ctrl = dlg.child_window(title=params.title, control_type=win_role)
            elif params.title:
                ctrl = dlg.child_window(title=params.title)
            elif win_role:
                ctrl = dlg.child_window(control_type=win_role)
            else:
                raise ValueError("Provide 'title' or 'role' for get_value.")
            return ctrl.window_text()
        except Exception as exc:
            raise RuntimeError(f"Could not get value of '{params.title or params.role}': {exc}") from exc

    raise ValueError(f"Unknown action: {params.action!r}")
