# opencua

**Open Computer Use Agent** — gives any AI agent eyes and hands on your desktop.

opencua runs as an MCP server. Install it, register it with your agent tool, and it adds screenshot, accessibility-based UI control, mouse, keyboard, clipboard, and OCR to every conversation — on macOS, Linux, and Windows.

---

## Quickstart

```bash
pip install 'opencua[core,mcp]'
```

**Claude Code** — one command:
```bash
claude mcp add opencua -- opencua-mcp
```

**Claude Desktop** — add to `~/Library/Application Support/Claude/claude_desktop_config.json`:
```json
{
  "mcpServers": {
    "opencua": { "command": "opencua-mcp" }
  }
}
```

**Cursor / Continue** — same pattern, point `command` at `opencua-mcp`.

That's it. Your agent can now say "take a screenshot", "click the Save button", or "type Hello World into TextEdit" and it will work.

---

## What it adds to your agent

| Tool | What it does |
|------|-------------|
| `ui` | Clicks buttons, types text, reads values — by element name, no pixel coordinates. Uses the platform's native accessibility tree (AppleScript / AT-SPI2 / UI Automation). |
| `screenshot` | Captures the screen. With `marks=True`, overlays numbered boxes on every interactive element so the agent can say "click mark 3". |
| `mouse` | Pixel-level mouse control with automatic Retina/HiDPI scaling. Last resort when `ui` has nothing to click. |
| `keyboard` | Types text (full Unicode), presses keys, sends hotkeys. |
| `app` | Opens, closes, and focuses applications. |
| `clipboard` | Reads and writes the system clipboard. |
| `ocr` | Extracts text from any screen region without sending to the LLM. |

**The agent follows a natural priority:** `ui` first (no coordinates needed) → `screenshot(marks=True)` to see numbered elements → `mouse` as last resort for unlabelled canvas areas.

---

## How the MCP integration works

```
Claude Code / Claude Desktop / Cursor / Continue
          |
          | MCP stdio
          v
     opencua-mcp
          |
          +-- screenshot, ui, mouse, keyboard, app, clipboard, ocr
```

opencua starts as a child process, speaks the MCP protocol over stdin/stdout, and the LLM client handles all tool-calling automatically. You never write tool-calling code.

---

## Why opencua?

- **MCP-first** — works out of the box with any MCP client, zero glue code.
- **Accessibility tree first** — the `ui` tool interacts with apps the same way a screen reader does, without pixel coordinates or Retina scaling headaches.
- **Framework-agnostic** — also ships Anthropic SDK, OpenAI, and LangChain adapters.
- **Sandboxed** — per-session audit log, app allow-list, screen region constraints.
- **Extensible** — one class to add a custom tool; it appears in all integrations automatically.

---

## Installation

```bash
# Minimal (just the framework, no hardware deps)
pip install opencua

# Core computer use: screen capture + mouse/keyboard
pip install 'opencua[core]'

# With MCP server support
pip install 'opencua[core,mcp]'

# Everything
pip install 'opencua[all]'
```

### System dependencies

| Platform | Required |
|----------|---------|
| macOS    | Screen Recording permission (System Settings → Privacy → Screen Recording); Accessibility permission for mouse/keyboard |
| Linux    | `xclip` for clipboard; `xdotool` or `pyatspi` for keyboard/UI |
| Windows  | No extra system deps (uses Win32 APIs) |

---

## Quick start

```python
import asyncio
from opencua import create_registry, allow_all_context

async def main():
    registry = create_registry()
    ctx = allow_all_context()

    # Take a screenshot with Set-of-Marks overlay
    screenshot = registry.get("screenshot")
    result = await screenshot.execute(ctx, screenshot.Params(marks=True))
    print(result.output)   # lists all interactive elements as [1] Button "OK" ...
    # result.attachments[0].content  -> PNG bytes

    # Click a button by name — no pixel coordinates needed
    ui = registry.get("ui")
    await ui.execute(ctx, ui.Params(action="click", app="Safari", title="Go"))

    # Type text
    kb = registry.get("keyboard")
    await kb.execute(ctx, kb.Params(action="type", text="hello world"))

asyncio.run(main())
```

---

## Integrations

### MCP server (Claude Desktop, Continue, Cursor, ...)

Run the MCP server over stdio:

```bash
opencua-mcp
```

Add to Claude Desktop (`~/Library/Application Support/Claude/claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "opencua": {
      "command": "opencua-mcp"
    }
  }
}
```

Or create a server in Python:

```python
from opencua.integrations.mcp import create_mcp_server
from opencua.registry import create_registry
from mcp.server.stdio import stdio_server

server = create_mcp_server(create_registry())
async with stdio_server() as (r, w):
    await server.run(r, w, server.create_initialization_options())
```

---

### Claude Code / Anthropic SDK

```python
import anthropic
from opencua.integrations.claude_code import ClaudeCodeAdapter
from opencua.registry import create_registry

client = anthropic.Anthropic()
adapter = ClaudeCodeAdapter(create_registry())

messages = [{"role": "user", "content": "Open Safari and take a screenshot"}]

# Full agentic loop (handles tool use automatically)
final_text = await adapter.run_loop(
    client,
    model="claude-opus-4-6",
    messages=messages,
    system="You are a computer use agent. Use the ui tool first, mouse as last resort.",
)
print(final_text)
```

Manual control:

```python
response = client.messages.create(
    model="claude-opus-4-6",
    max_tokens=4096,
    tools=adapter.tool_definitions(),
    messages=messages,
)

# Dispatch all tool_use blocks in parallel
tool_results = await adapter.handle_response(response)
messages.append({"role": "assistant", "content": response.content})
messages.append({"role": "user", "content": tool_results})
```

---

### OpenAI function calling

Works with OpenAI, Groq, Together AI, Ollama, LiteLLM, and any OpenAI-compatible provider:

```python
from openai import OpenAI
from opencua.integrations.openai_compat import OpenAIAdapter
from opencua.registry import create_registry

client = OpenAI()
adapter = OpenAIAdapter(create_registry())

messages = [{"role": "user", "content": "Take a screenshot"}]
final_text = await adapter.run_loop(client, model="gpt-4o", messages=messages)
```

---

### LangChain / LangGraph

```python
from langchain_openai import ChatOpenAI
from langgraph.prebuilt import create_react_agent

from opencua.integrations.langchain_compat import as_langchain_tools
from opencua.registry import create_registry

tools = as_langchain_tools(create_registry())
agent = create_react_agent(ChatOpenAI(model="gpt-4o"), tools)
result = agent.invoke({"messages": [("user", "Take a screenshot")]})
```

---

## Tools

| Tool | Description |
|------|-------------|
| `ui` | Click, type, and read UI elements by name via the accessibility tree. **Use this first.** |
| `screenshot` | Capture screen with Set-of-Marks overlay, cursor dot, zoom, and change detection |
| `mouse` | Click, scroll, drag — with automatic Retina/HiDPI coordinate scaling |
| `keyboard` | Type (full Unicode), press keys, hotkeys, hold |
| `app` | Open, close, focus, or list applications |
| `clipboard` | Read or write the system clipboard |
| `ocr` | Extract text from any screen region (pytesseract / Vision / WinRT) |

### Tool priority

When the agent needs to interact with a UI element:

1. **`ui` tool** — click by element title/role, no coordinates needed. Most reliable.
2. **`screenshot` with `marks=True`** — if `ui` doesn't find the element, get a SoM overlay showing numbered bounding boxes.
3. **`mouse` with `image_width`/`image_height`** — last resort for unlabelled canvas areas. Always provide image dimensions for correct Retina scaling.

---

## Architecture

```
opencua/
├── tools/          # Tool definitions (base.py + one file per tool)
├── computer/       # Low-level helpers: capture, marks (SoM), OCR, sandbox
├── integrations/   # MCP, Claude Code, OpenAI, LangChain adapters
└── registry.py     # ToolRegistry + create_registry()
```

See [docs/architecture.md](docs/architecture.md) for a deep dive.

---

## Permission model

Every tool action goes through a `ToolContext.check_permission()` call before execution.

```python
from opencua.tools.base import allow_all_context, interactive_context

# Headless / autonomous — approve everything automatically
ctx = allow_all_context()

# Interactive — prompt on stdout before each action
ctx = interactive_context()

# Custom handler — integrate with your own UI or policy engine
async def my_handler(tool: str, argument: str, description: str) -> None:
    if "production" in description.lower():
        raise PermissionDeniedError("Refusing to act on production.")

from opencua.tools.base import ToolContext
ctx = ToolContext(session_id="my-session", permission_handler=my_handler)
```

---

## Platform support

| Feature | macOS | Linux | Windows |
|---------|-------|-------|---------|
| Screenshot | mss + Pillow | mss + Pillow | mss + Pillow |
| Mouse control | pyautogui | pyautogui | pyautogui |
| Keyboard (Unicode) | pbcopy + cmd+v | xclip/xsel + ctrl+v | pyperclip + ctrl+v |
| AX tree (`ui` tool) | AppleScript | AT-SPI2 / xdotool | pywinauto |
| SoM marks | AppleScript | pyatspi | pywinauto |
| OCR | pytesseract / Vision | pytesseract | pytesseract / WinRT |
| App open/close | `open -a` / AppleScript | xdg-open / pkill | start / taskkill |

---

## License

MIT
