# Architecture

## Overview

opencua is structured in three layers:

```
┌─────────────────────────────────────────────────────────┐
│  Integrations  (MCP, Claude Code, OpenAI, LangChain)    │
├─────────────────────────────────────────────────────────┤
│  Tools         (screenshot, mouse, keyboard, ui, ...)   │
├─────────────────────────────────────────────────────────┤
│  Computer      (capture, marks/SoM, OCR, sandbox)       │
└─────────────────────────────────────────────────────────┘
```

Each layer is independently importable. A caller that only wants the MCP server doesn't need to import any tool internals.

---

## Layer 1: computer/

Low-level modules with no tool or integration dependencies.

### `capture.py`
Screen capture via `mss`. Key details:
- `mss` returns BGRA pixels; PIL's `"BGRX"` raw decoder reorders them to RGB correctly.
- Screens wider than 1920 px are downscaled to stay under LLM image-size limits.
- `diff_screenshots()` uses `ImageChops.difference()` and a threshold mask to detect changed regions between two captures. Reports `change_fraction` and a bounding box.

### `marks.py` — Set-of-Marks (SoM)
Implements the SoM visual prompting technique (Yang et al. 2023 / OmniParser / AgentS).

1. `get_interactive_elements()` queries the platform's native accessibility API:
   - macOS: `osascript` / AppleScript — queries `System Events` for `AXButton`, `AXTextField`, etc.
   - Linux: `pyatspi` (AT-SPI2) — walks the AT-SPI tree filtering interactive roles.
   - Windows: `pywinauto` UI Automation — enumerates `descendants()` filtering by friendly class name.
   Returns `[{mark, role, label, x, y, w, h}]` in logical screen coordinates.

2. `draw_som_marks()` overlays numbered, colour-coded chips on a PIL Image. Scale factors (`scale_x = screenshot_width / logical_screen_width`) handle Retina/HiDPI correctly.

3. `overlay_cursor()` draws a red disc + white dot at the current cursor position, matching Anthropic's computer-use demo style.

### `ocr.py`
Text extraction from screen regions. Three backends tried in order:
1. `pytesseract` — best quality, cross-platform.
2. macOS Vision (Swift subprocess, zero deps, macOS 11+).
3. Windows WinRT OCR (PowerShell, zero deps, Windows 10+).

### `sandbox.py`
Per-session audit log and policy enforcement.
- `ComputerSandbox` tracks every action with a UUID, timestamp, params, and result.
- `allowed_apps` — app name allow-list for open/close/focus actions.
- `screen_region` — bounding box constraint; coordinates outside it are rejected.
- `last_screenshot` — stores the previous PNG for automatic change detection.
- `export_audit_log()` — returns the full log as plain dicts for compliance export.

---

## Layer 2: tools/

### `base.py`
Core abstractions:

```
ToolResult      — title, output, error, attachments, metadata
Attachment      — filename, content (bytes), media_type
ToolContext     — session_id, permission_handler (async callable)
Tool            — ABC: name, description, Params (Pydantic), execute()
```

`ToolContext.check_permission(tool, argument, description)` calls the injected handler before every action. If no handler is set, all actions are approved. If the handler raises `PermissionDeniedError`, the tool returns an error result.

`Tool.get_schema()` returns the JSON Schema for `Params` via Pydantic's `model_json_schema()`. This is the single source of truth used by all integrations to generate tool definitions.

### Tool files

Each tool file contains one class inheriting from `Tool`:

| File | Class | Key dependencies |
|------|-------|-----------------|
| `screenshot.py` | `ScreenshotTool` | `opencua.computer.capture`, `marks` |
| `mouse.py` | `MouseTool` | `pyautogui` |
| `keyboard.py` | `KeyboardTool` | `pyautogui`, `pbcopy`/`xclip`/`pyperclip` |
| `app.py` | `AppTool` | `osascript`/`xdg-open`/`start` |
| `ui.py` | `UITool` | `osascript`/`pyatspi`/`pywinauto` |
| `clipboard.py` | `ClipboardTool` | `pbcopy`/`xclip`/`pyperclip` |
| `ocr.py` | `OCRTool` | `opencua.computer.ocr` |

All blocking I/O runs in `asyncio.get_event_loop().run_in_executor(None, ...)` so tools are safe to call from async code without blocking the event loop.

### UITool — why it's the primary interaction path

The `ui` tool uses each platform's accessibility API to find elements by their visible label, not their pixel location. This means:
- No coordinate guessing.
- No Retina scaling translation.
- Works regardless of window position, screen resolution, or zoom level.
- Provides descriptive errors: `"button 'Save' not found in TextEdit"`.

Mouse coordinates are only needed for elements with no accessible label (canvas games, video players, drawing apps).

---

## Layer 3: integrations/

### `mcp.py`
Wraps a `ToolRegistry` as an MCP `Server`. On `list_tools`, returns tool definitions built from each tool's JSON schema. On `call_tool`, parses arguments, calls `tool.execute()`, and converts `ToolResult` attachments to `ImageContent` blocks.

### `claude_code.py`
Converts tool schemas to Anthropic's `input_schema` format. `ClaudeCodeAdapter.run_loop()` drives the standard `stop_reason == "tool_use"` loop, dispatching all tool_use blocks in parallel via `asyncio.gather`.

### `openai_compat.py`
Wraps schemas in OpenAI's `{"type": "function", "function": {...}}` envelope. Handles `tool_calls` from `chat.completions.create` responses. Compatible with any OpenAI-format API.

### `langchain_compat.py`
Creates `BaseTool` subclasses dynamically. Both `_run` (sync) and `_arun` (async) are implemented.

---

## Data flow

```
User / LLM
    │
    │ tool_name + arguments (dict)
    ▼
ToolRegistry.get(name)
    │
    ▼
Tool.parse_params(arguments)   ← Pydantic validation
    │
    ▼
ToolContext.check_permission() ← policy gate
    │
    ▼
Tool.execute(ctx, params)
    │
    ├── computer layer (capture / marks / OCR)
    ├── sandbox.record_action()
    │
    ▼
ToolResult { output, attachments, error }
    │
    ▼
Integration adapter (MCP / Anthropic / OpenAI / LangChain)
    │
    ▼
LLM response
```

---

## Adding a custom tool

```python
from opencua.tools.base import Tool, ToolContext, ToolResult
from opencua.registry import create_registry
from pydantic import Field

class PingTool(Tool):
    name = "ping"
    description = "Check if a host is reachable."

    class Params(Tool.Params):
        host: str = Field(description="Hostname or IP to ping.")
        count: int = Field(default=3, description="Number of pings.")

    async def execute(self, ctx: ToolContext, params: "PingTool.Params") -> ToolResult:
        import asyncio
        proc = await asyncio.create_subprocess_exec(
            "ping", "-c", str(params.count), params.host,
            stdout=asyncio.subprocess.PIPE,
        )
        stdout, _ = await proc.communicate()
        return ToolResult(
            title=f"Ping {params.host}",
            output=stdout.decode(),
            error=proc.returncode != 0,
        )

# Register and use
registry = create_registry()
registry.register(PingTool())
```

It will automatically appear in MCP, Anthropic, OpenAI, and LangChain adapters.
