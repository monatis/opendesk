# Tools Reference

All tools share the same interface: `await tool.execute(ctx, params) -> ToolResult`.

**Tool priority rule:** `ui` → `screenshot(marks=True)` → `mouse` with image dimensions.

---

## `ui` — Accessibility-based UI interaction

The primary interaction tool. Clicks, types, and reads values using the platform's native accessibility API. No pixel coordinates needed.

```python
from opendesk.tools.ui import UITool
tool = UITool()
```

### Actions

| Action | Required params | Description |
|--------|----------------|-------------|
| `get_tree` | `app` | List all accessible elements in the app window |
| `click` | `app`, `title` or `role` | Click a button or element by its visible label |
| `click_menu` | `app`, `menu`, `menu_item` | Click a menu bar item: `File → Save` |
| `type` | `app`, `text` | Type text into the focused element (Unicode-safe) |
| `press_key` | `app`, `key` | Press a key or chord |
| `get_value` | `app`, `title` or `role` | Read the current text value of an element |

### Examples

```python
params = UITool.Params

# List what's in the window
await tool.execute(ctx, params(action="get_tree", app="TextEdit"))

# Click a button
await tool.execute(ctx, params(action="click", app="TextEdit", title="Save"))

# Open File menu → New
await tool.execute(ctx, params(action="click_menu", app="TextEdit", menu="File", menu_item="New"))

# Type text (clipboard-paste, full Unicode)
await tool.execute(ctx, params(action="type", app="TextEdit", text="Hello, 世界 🌍"))

# Press Cmd+S to save
await tool.execute(ctx, params(action="press_key", app="TextEdit", key="s", modifiers=["command"]))

# Read a text field value
await tool.execute(ctx, params(action="get_value", app="Safari", title="Address and Search Bar"))
```

### Platform notes

- **macOS**: uses AppleScript / System Events. App name must match Activity Monitor (e.g. `"Google Chrome"` not `"chrome"`).
- **Linux**: uses AT-SPI2 (`pyatspi`) with `xdotool` fallback for type/press_key.
- **Windows**: uses UI Automation with Win32 fallback. App name matches window title or process name.

When an app uses custom rendering (canvas apps, games, Electron), `get_tree` may return an empty tree — fall back to `screenshot(marks=True)` and the `mouse` tool.

---

## `screenshot` — Screen capture

```python
from opendesk.tools.screenshot import ScreenshotTool
tool = ScreenshotTool()
```

### Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `marks` | bool | false | Overlay numbered bounding boxes (Set-of-Marks) on interactive elements |
| `show_cursor` | bool | false | Draw a red dot at the current cursor position |
| `zoom` | `[x0,y0,x1,y1]` | null | Crop a region for close-up inspection |
| `region` | `[x,y,w,h]` | null | Capture only this screen region |
| `save_path` | str | null | Save PNG to disk at this absolute path |

### Examples

```python
params = ScreenshotTool.Params

# Full screenshot
result = await tool.execute(ctx, params())
png = result.attachments[0].content  # bytes

# With SoM overlay — model can say "click mark 3"
result = await tool.execute(ctx, params(marks=True))
print(result.output)  # lists: [1] Button "OK" at (120,340) 80×30 — click mark 1 to interact

# Zoom into a region
result = await tool.execute(ctx, params(zoom=[100, 200, 400, 350]))

# Save to disk
result = await tool.execute(ctx, params(save_path="/tmp/before.png"))
```

### Output

The tool output always includes:
- Capture dimensions and Retina note: `image_width=1440, image_height=900` to pass to the mouse tool.
- Change detection vs previous screenshot: `"12.3% of pixels changed in region [x=400, y=200, 600×300px]"`.
- SoM summary if `marks=True`.

---

## `mouse` — Mouse control

```python
from opendesk.tools.mouse import MouseTool
tool = MouseTool()
```

**Always provide `image_width` and `image_height` from the screenshot tool.** This ensures correct coordinate translation on Retina / HiDPI displays.

### Actions

| Action | Parameters | Description |
|--------|-----------|-------------|
| `click` | `x, y` | Left-click |
| `double_click` | `x, y` | Double left-click |
| `triple_click` | `x, y` | Triple-click (select word/line) |
| `right_click` | `x, y` | Right-click |
| `middle_click` | `x, y` | Middle-click (open link in new tab) |
| `move` | `x, y` | Move without clicking |
| `scroll` | `x, y, direction, amount` | Scroll up/down/left/right |
| `drag` | `x, y, end_x, end_y` | Click-and-drag |
| `left_down` | `x, y` | Press and hold left button |
| `left_up` | `x, y` | Release left button |
| `cursor_position` | — | Return current cursor location |

### Examples

```python
params = MouseTool.Params

# Always get screenshot dimensions first
shot = await registry.get("screenshot").execute(ctx, ScreenshotTool.Params())
w = shot.metadata["width"]
h = shot.metadata["height"]

# Click at image coordinates (auto-scaled to logical coords)
await tool.execute(ctx, params(action="click", x=400, y=300, image_width=w, image_height=h))

# Scroll down
await tool.execute(ctx, params(action="scroll", x=760, y=400, direction="down", amount=5,
                               image_width=w, image_height=h))

# Drag
await tool.execute(ctx, params(action="drag", x=100, y=100, end_x=500, end_y=300,
                               image_width=w, image_height=h))
```

---

## `keyboard` — Keyboard input

```python
from opendesk.tools.keyboard import KeyboardTool
tool = KeyboardTool()
```

### Actions

| Action | Required | Description |
|--------|----------|-------------|
| `type` | `text` | Type text (clipboard-paste, full Unicode including CJK and emoji) |
| `press` | `key` | Press a named key: `enter`, `escape`, `tab`, `f5`, etc. |
| `hotkey` | `keys` | Send a key combination: `["ctrl","c"]`, `["command","s"]` |
| `hold` | `key`, `hold_duration` | Hold a key for N seconds then release |

### Examples

```python
params = KeyboardTool.Params

# Type text
await tool.execute(ctx, params(action="type", text="Hello, 世界! 🌍"))

# Press Enter
await tool.execute(ctx, params(action="press", key="enter"))

# Copy (Ctrl+C)
await tool.execute(ctx, params(action="hotkey", keys=["ctrl", "c"]))

# Cmd+Shift+P (VS Code command palette on macOS)
await tool.execute(ctx, params(action="hotkey", keys=["command", "shift", "p"]))

# Hold Shift for 0.5s (e.g. while clicking for range select)
await tool.execute(ctx, params(action="hold", key="shift", hold_duration=0.5))
```

Key names follow pyautogui conventions: `enter`, `escape`, `tab`, `backspace`, `delete`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `f1`–`f12`, `ctrl`, `alt`, `shift`, `cmd`/`win`, etc.

---

## `app` — Application control

```python
from opendesk.tools.app import AppTool
tool = AppTool()
```

| Action | Parameters | Description |
|--------|-----------|-------------|
| `open` | `name` | Launch application by name or path |
| `close` | `name` | Quit application |
| `focus` | `name` | Bring window to foreground |
| `list` | — | List all running applications |

```python
params = AppTool.Params

await tool.execute(ctx, params(action="open", name="TextEdit"))
await tool.execute(ctx, params(action="focus", name="Safari"))
result = await tool.execute(ctx, params(action="list"))
print(result.output)
await tool.execute(ctx, params(action="close", name="TextEdit"))
```

---

## `clipboard` — Clipboard read/write

```python
from opendesk.tools.clipboard import ClipboardTool
tool = ClipboardTool()
```

| Action | Parameters | Description |
|--------|-----------|-------------|
| `read` | — | Return current clipboard text |
| `write` | `text` | Set clipboard text |

```python
params = ClipboardTool.Params

# Read
result = await tool.execute(ctx, params(action="read"))
print(result.output)

# Write, then paste
await tool.execute(ctx, params(action="write", text="Hello from opendesk"))
# Then use keyboard tool to paste with Ctrl/Cmd+V
```

---

## `ocr` — Text extraction

```python
from opendesk.tools.ocr import OCRTool
tool = OCRTool()
```

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `region` | `[x,y,w,h]` | null | Screen region to OCR; null = full screen |

```python
params = OCRTool.Params

# Full screen
result = await tool.execute(ctx, params())
print(result.output)

# Specific region (e.g. a dialog box)
result = await tool.execute(ctx, params(region=[400, 200, 600, 300]))
```

Backends tried in order: `pytesseract` → macOS Vision (macOS only) → Windows WinRT (Windows only) → install hint if none available.

---

## ToolResult

All tools return:

```python
@dataclass
class ToolResult:
    title: str           # short human-readable label
    output: str          # text returned to the LLM
    error: bool          # True if the action failed
    attachments: list[Attachment]  # binary files (screenshots, etc.)
    metadata: dict       # extra data for programmatic consumers
```

`Attachment`:

```python
@dataclass
class Attachment:
    filename: str
    content: bytes
    media_type: str      # e.g. "image/png"

    def to_base64(self) -> str: ...
```

---

## `learn` — Record and replay tasks

```python
from opendesk.tools.learn import LearnTool
tool = LearnTool()
```

Requires `pip install 'opendesk[learn]'` (installs `pynput`).

### Actions

| Action | Parameters | Description |
|--------|-----------|-------------|
| `start` | `task_name` | Begin recording mouse, keyboard, and screenshots globally |
| `stop` | — | Stop recording; returns trajectory summary and screenshots |
| `save` | `task_name`, `procedure` | Save a procedure JSON string to `.opendesk/learned/` |
| `replay` | `task_name` | Load a procedure and return step-by-step replay instructions |
| `list` | — | List all saved procedures in the current directory |

### Examples

```python
params = LearnTool.Params

# Start recording
await tool.execute(ctx, params(action="start", task_name="fill-form"))

# ... user performs the task ...

# Stop and review trajectory
result = await tool.execute(ctx, params(action="stop"))
print(result.output)

# Save procedure (JSON string)
import json
procedure = json.dumps({
    "task_name": "fill-form",
    "description": "Fill and submit the expense form",
    "steps": ["Open the form", "Fill in fields", "Click Submit"],
    "procedure": "Navigate to the form application. Fill each required field. Submit."
})
await tool.execute(ctx, params(action="save", task_name="fill-form", procedure=procedure))

# Replay
result = await tool.execute(ctx, params(action="replay", task_name="fill-form"))
print(result.output)  # step-by-step instructions for the agent

# List all
result = await tool.execute(ctx, params(action="list"))
print(result.output)
```

See [learn.md](learn.md) for a full guide including accessibility context, storage format, and tips.
