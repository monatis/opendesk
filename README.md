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

---

## What your agent can do

| Tool | What it does |
|------|-------------|
| `screenshot` | See the screen — with numbered boxes on every clickable element |
| `ui` | Click buttons and type text by name, no pixel coordinates needed |
| `mouse` | Pixel-level mouse control for anything `ui` can't reach |
| `keyboard` | Type text, press keys, send hotkeys |
| `app` | Open, close, and focus applications |
| `clipboard` | Read and write the system clipboard |
| `ocr` | Extract text from any part of the screen |
| `learn` | Record a task once, replay it anytime |

---

## Record and replay tasks

Teach your agent to repeat any workflow:

1. Say **"start recording task fill-expense-form"**
2. Do the task yourself
3. Say **"stop recording"** — the agent summarizes it into a reusable procedure
4. Next time, say **"replay fill-expense-form"** — the agent follows the steps automatically

Procedures are saved in `.opendesk/learned/` in your project directory.

---

## System permissions (macOS)

Go to **System Settings → Privacy & Security** and enable:
- **Screen Recording** — for screenshots
- **Accessibility** — for mouse and keyboard control

---

## Other integrations

### Claude Desktop

Add to `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "opendesk": { "command": "opendesk-mcp" }
  }
}
```

Restart Claude Desktop.

### Cursor / Continue

Point `command` at `opendesk-mcp` in your MCP config.

---

## Use in Python

```python
import asyncio
from opendesk import create_registry, allow_all_context

async def main():
    registry = create_registry()
    ctx = allow_all_context()

    screenshot = registry.get("screenshot")
    result = await screenshot.execute(ctx, screenshot.Params(marks=True))
    print(result.output)  # lists all interactive elements

    ui = registry.get("ui")
    await ui.execute(ctx, ui.Params(action="click", app="Safari", title="Go"))

asyncio.run(main())
```

Works with Anthropic SDK, OpenAI, and LangChain — see [docs/integrations.md](docs/integrations.md).

---

## Installation options

```bash
pip install opendesk                    # framework only
pip install 'opendesk[core]'            # + screen capture, mouse, keyboard
pip install 'opendesk[core,mcp]'        # + MCP server (recommended)
pip install 'opendesk[core,mcp,learn]'  # + task recording
pip install 'opendesk[all]'             # everything
```

---

## License

MIT
