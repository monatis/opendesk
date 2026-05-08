# MCP Integration

MCP (Model Context Protocol) is how AI coding tools like Claude Code, Cursor, and Continue
connect to external capabilities. opendesk ships as an MCP server — you install it once, register
it with your client, and from then on any conversation can use screenshot, mouse, keyboard, ui,
app, clipboard, and ocr without any extra setup.

---

## How it works

```
Your AI client (Claude Code / Cursor / Continue)
       |
       | MCP over stdio
       v
  opendesk-mcp  <-- the server this package installs
       |
       +-- screenshot tool  (captures screen, returns PNG to the LLM)
       +-- ui tool          (clicks elements by name via AX tree)
       +-- mouse tool       (pixel-level mouse control with Retina scaling)
       +-- keyboard tool    (types text, presses keys, hotkeys)
       +-- app tool         (opens, closes, focuses apps)
       +-- clipboard tool   (read/write clipboard)
       +-- ocr tool         (extracts text from any screen region)
```

The LLM decides when to call a tool, calls it, gets the result (including PNG screenshots),
and continues. You never write tool-calling code yourself.

---

## Step 1 — Install

```bash
pip install 'opendesk[core,mcp]'
```

Verify the server command exists:

```bash
opendesk-mcp --help    # should print usage or hang (waiting for MCP handshake on stdin)
# Ctrl-C to exit
```

---

## Step 2 — Register with your client

### Claude Code

```bash
claude mcp add opendesk -- opendesk-mcp
```

Verify it was added:

```bash
claude mcp list
# opendesk: opendesk-mcp
```

Now start a conversation in Claude Code and ask:
> "Take a screenshot and tell me what's on my screen."

Claude will call the `screenshot` tool, receive the PNG, and describe it.

To remove it later:

```bash
claude mcp remove opendesk
```

---

### Claude Desktop

Edit `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "opendesk": {
      "command": "opendesk-mcp"
    }
  }
}
```

Restart Claude Desktop. The tools appear in the toolbar.

If `opendesk-mcp` is not on your PATH (e.g. in a virtualenv), use the full path:

```json
{
  "mcpServers": {
    "opendesk": {
      "command": "/path/to/venv/bin/opendesk-mcp"
    }
  }
}
```

---

### Cursor

Create or edit `.cursor/mcp.json` in your project root:

```json
{
  "mcpServers": {
    "opendesk": {
      "command": "opendesk-mcp",
      "transport": "stdio"
    }
  }
}
```

---

### Continue (VS Code extension)

Edit `.continue/config.json`:

```json
{
  "mcpServers": [
    {
      "name": "opendesk",
      "command": "opendesk-mcp",
      "transport": "stdio"
    }
  ]
}
```

---

### Any other MCP client

The pattern is always the same — point the client at the `opendesk-mcp` command with stdio transport.
opendesk follows the MCP spec exactly, so it works with any compliant client.

---

## What the LLM sees

Once registered, the LLM sees these tools in every conversation:

| Tool | When the LLM uses it |
|------|---------------------|
| `screenshot` | "Let me see what's on the screen" |
| `ui` | "Click the Save button" (finds it via AX tree, no coordinates) |
| `mouse` | Last resort for canvas areas with no accessible elements |
| `keyboard` | "Type this text into the field" |
| `app` | "Open Terminal" / "List running apps" |
| `clipboard` | "Copy this to clipboard" / "What did I copy?" |
| `ocr` | "Read the text in that region" |

The LLM follows the priority rule automatically because it's in each tool's description:
`ui` first, then `screenshot(marks=True)` to see numbered elements, then `mouse` as last resort.

---

## Running the MCP server in Python (advanced)

If you need to embed the server in your own process or customize permissions:

```python
import asyncio
from mcp.server.stdio import stdio_server
from opendesk.integrations.mcp import create_mcp_server
from opendesk.registry import create_registry
from opendesk.tools.base import ToolContext, PermissionDeniedError

# Custom permission policy: block any app launch
async def my_policy(tool: str, argument: str, description: str) -> None:
    if tool == "app" and "open" in argument:
        raise PermissionDeniedError("App launching is disabled in this session.")

ctx = ToolContext(session_id="my-session", permission_handler=my_policy)
server = create_mcp_server(registry=create_registry(), ctx=ctx)

async def main():
    async with stdio_server() as (r, w):
        await server.run(r, w, server.create_initialization_options())

asyncio.run(main())
```

---

## Restricting what the agent can do

You can lock down the sandbox before starting the server:

```python
from opendesk.computer.sandbox import configure_sandbox
from opendesk.tools.base import ToolContext

# Only allow Safari and Terminal; only act within the left half of the screen
configure_sandbox(
    session_id="restricted",
    allowed_apps=["Safari", "Terminal"],
    screen_region=(0, 0, 1280, 1600),
)

ctx = ToolContext(session_id="restricted")
server = create_mcp_server(ctx=ctx)
```

All screenshots and mouse clicks outside `(0,0,1280,1600)` will be rejected with an error
the LLM sees, so it knows to stay in bounds.

---

## Audit log

Every action is recorded in the session sandbox:

```python
from opendesk.computer.sandbox import get_sandbox

sandbox = get_sandbox("my-session")
print(sandbox.summary())
# session=my-session… | 12 actions: 3x screenshot, 4x ui_action, 2x keyboard_type, 3x mouse_click

log = sandbox.export_audit_log()
# [{"id": "...", "timestamp": 1234567890.0, "action": "screenshot", "params": {...}, ...}, ...]
```

---

## Troubleshooting

**`opendesk-mcp` not found**
The command is registered as a script entry point. Make sure you installed with `pip install 'opendesk[core,mcp]'`
and that the install's `bin/` directory is on your PATH.

**Screenshot returns a permission error on macOS**
Go to System Settings -> Privacy & Security -> Screen Recording and add your terminal app (Terminal, iTerm2, etc.).

**Mouse/keyboard actions do nothing on macOS**
Go to System Settings -> Privacy & Security -> Accessibility and add your terminal app.

**Tools appear in Claude Code but calls fail with `ImportError`**
You installed opendesk but not the core extras. Run: `pip install 'opendesk[core]'`
