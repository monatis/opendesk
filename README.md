# opendesk

**Give any AI agent eyes and hands on your desktop.**

opendesk connects to Claude Code, Claude Desktop, Cursor, and Continue via MCP — adding screenshot, click, type, scroll, clipboard, OCR, and task recording to every conversation. Works on macOS, Linux, and Windows.

---

## Install in 2 steps

```bash
pip install 'opendesk[core,mcp]'
opendesk install
```

That's it. Start a Claude Code conversation and say:

> "Take a screenshot"
> "Click the Save button"
> "Fill in this form"
> "Start recording this task"

> **Requires Python 3.10+**

---

## What your agent can do

| Tool | What it does |
|------|-------------|
| `screenshot` | See the screen — with numbered boxes on every clickable element |
| `ui` | Click buttons and type text by element name, no coordinates needed |
| `mouse` | Pixel-level mouse control for anything `ui` can't reach |
| `keyboard` | Type text, press keys, send hotkeys |
| `app` | Open, close, and focus applications |
| `clipboard` | Read and write the system clipboard |
| `ocr` | Extract text from any part of the screen |
| `learn` | Record a task once, replay it anytime |

---

## Record and replay tasks

The `learn` tool lets your agent memorize any workflow and repeat it on demand.

**1. Start recording**
> "Start recording task expense-form"

The agent begins capturing every mouse click, keystroke, and screenshot.

**2. Do the task yourself**

Perform the workflow normally — fill the form, navigate the UI, click through steps.

**3. Stop and save**
> "Stop recording"

The agent summarizes the recording into a reusable procedure and saves it to `.opendesk/learned/` in your project directory.

**4. Replay anytime**
> "Replay expense-form"

The agent loads the saved procedure and re-executes it using the available tools — adapting to the current screen state automatically.

---

## System permissions

### macOS
- **System Settings → Privacy & Security → Screen Recording** — enable for your terminal app
- **System Settings → Privacy & Security → Accessibility** — enable for mouse and keyboard control

### Linux
```bash
# Clipboard support
sudo apt install xclip

# UI automation
sudo apt install xdotool python3-atspi
```

### Windows
No extra permissions needed — opendesk uses Win32 APIs by default.

---

## Other integrations

### Claude Desktop

Add to your Claude Desktop config:

- **macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Windows**: `%APPDATA%\Claude\claude_desktop_config.json`
- **Linux**: `~/.config/Claude/claude_desktop_config.json`

```json
{
  "mcpServers": {
    "opendesk": { "command": "opendesk-mcp" }
  }
}
```

Restart Claude Desktop.

### Cursor / Continue

Point `command` at `opendesk-mcp` in your MCP config file.

---

## Use in Python

```python
import asyncio
from opendesk import create_registry, allow_all_context

async def main():
    registry = create_registry()
    ctx = allow_all_context()

    # Take a screenshot with numbered elements
    screenshot = registry.get("screenshot")
    result = await screenshot.execute(ctx, screenshot.Params(marks=True))
    print(result.output)

    # Click a button by name — no coordinates needed
    ui = registry.get("ui")
    await ui.execute(ctx, ui.Params(action="click", app="Notepad", title="File"))

    # Type text
    kb = registry.get("keyboard")
    await kb.execute(ctx, kb.Params(action="type", text="Hello from opendesk"))

asyncio.run(main())
```

Works with Anthropic SDK, OpenAI, and LangChain — see [docs/integrations.md](docs/integrations.md).

---

## Installation options

```bash
pip install opendesk                         # framework only
pip install 'opendesk[core]'                 # + screen capture, mouse, keyboard
pip install 'opendesk[core,mcp]'             # + MCP server (recommended)
pip install 'opendesk[core,mcp,learn]'       # + task recording and replay
pip install 'opendesk[all]'                  # everything
```

---

## Platform support

| Feature | macOS | Linux | Windows |
|---------|-------|-------|---------|
| Screenshot | ✓ | ✓ | ✓ |
| Mouse control | ✓ | ✓ | ✓ |
| Keyboard | ✓ | ✓ | ✓ |
| UI element access | AppleScript | AT-SPI2 / xdotool | UI Automation |
| Clipboard | pbcopy/pbpaste | xclip / xsel | pyperclip |
| OCR | Vision / tesseract | tesseract | WinRT / tesseract |
| App open/close | open -a | xdg-open | start |
| Task recording | ✓ | ✓ | ✓ |

---

## License

MIT
