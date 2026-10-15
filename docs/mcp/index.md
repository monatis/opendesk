# MCP Integration

MCP (Model Context Protocol) is how AI coding tools like Claude Code, Cursor, and Continue
connect to external capabilities. opendesk ships as an MCP server — you install it once, register
it with your client, and from then on any conversation can use screenshot, mouse, keyboard, ui,
app, clipboard, and exec without any extra setup.

---

## How it works

```
Your AI client (Claude Code / Cursor / Continue)
       |
       | MCP over stdio
       v
  opendesk-mcp  (or `opendesk mcp`)
       |
       +-- screenshot tool    (captures screen as PNG/JPEG for LLM observation)
       +-- opendesk_view tool (opens interactive MCP App viewport inline in chat)
       +-- ui tool            (finds and clicks elements by name via the accessibility tree)
       +-- mouse tool         (pixel-level mouse control in screenshot coordinates)
       +-- keyboard tool      (types text, presses keys and chords, atomic sequences)
       +-- app tool           (opens, closes, focuses, lists apps)
       +-- clipboard tool     (read/write clipboard)
       +-- opendesk_exec tool (runs shell commands on this machine or a remote peer)
       +-- audit, learn, schedule, and opendesk_* admin tools
```

The LLM decides when to call a tool, calls it, gets the result (including screenshots),
and continues. You never write tool-calling code yourself.

---

## What the LLM sees

| Tool | When the LLM uses it |
|------|---------------------|
| `screenshot` | "Let me see what's on the screen" (PNG or JPEG with quality control) |
| `opendesk_view` | "Open the interactive remote desktop viewport so the user can watch live or take over" |
| `ui` | "Click the Save button" (finds it via the accessibility tree, no coordinates) |
| `mouse` | Canvas areas with no accessible elements |
| `keyboard` | "Type this text into the field" / chords such as `ctrl+shift+s` / sequences |
| `app` | "Open Notepad" / "List running apps" |
| `clipboard` | "Copy this to clipboard" / "What did I copy?" |
| `opendesk_exec` | "Run a command or script on the local or remote machine" |
| `audit` | "Show the audit log" |
| `learn`, `schedule` | Save / replay procedures; store schedules (not executed) |
| `opendesk_guard` | "Only send input while Notepad is in front" |
| `opendesk_peers`, `opendesk_use`, … | Choosing which machine to act on — see [Remote: MCP Integration](../remote/mcp.md) |

There is no `ocr` tool in the Rust version. Full parameters: [Tools reference](../tools/index.md).

---

## Ask Claude

Once connected, just describe what you want in plain English:

> "Take a screenshot and tell me what's on my screen"

> "Click the Save button in Notepad"

> "Open Notepad and type hello world, but only if Notepad is the foreground window"

> "Run `Get-Process` and tell me what's using the most memory"

> "Show me the audit log"

---

## Setup & Interfaces

1. [Install](install.md) the binaries
2. Register with your client: [Claude Code](claude-code.md) · [Claude Desktop](claude-desktop.md) · [Cursor](cursor.md) · [Continue](continue.md)
3. [MCP Apps (Interactive UI Viewport)](apps.md) — Embed interactive remote desktop viewports inside Claude Desktop, Goose, or custom harnesses like `isanagent`.
