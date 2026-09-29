# Rust Quickstart

OpenDesk is built entirely in Rust (`opendesk-rs`), delivering single-binary deployments, sub-millisecond execution times, and deep platform accessibility integration.

## Installation

Install directly from source via `cargo`:

```bash
cargo install --path opendesk-rs --force
```

This installs:
- `opendesk`: The core desktop automation CLI, background server, and pairing manager.
- `opendesk-mcp`: The stdio Model Context Protocol (MCP) server for AI agent harnesses.

## Register with Claude Code

Register OpenDesk globally as an MCP server for Claude Code with one command:

```bash
opendesk install
```

To remove the registration:

```bash
opendesk uninstall
```

## Try it via Claude Code or any MCP Client

Once registered, open Claude Code (or Cursor / Claude Desktop) and ask:

> "Take a screenshot and describe what's on my screen"

> "Open Notepad and type hello world"

> "Inspect the active window's accessibility tree"

> "Click the Save button"

> "Show me the audit log"

Your AI model interacts with OpenDesk via native MCP tool calls automatically.

---

## Remote Computer Use (LAN & Internet)

OpenDesk makes it effortless to control another machine remotely:

### 1. Controlled Machine
```bash
opendesk pair
```
Displays a one-time, PBKDF2-stretched 6-digit code.

### 2. Controller Machine
```bash
opendesk discover
opendesk pair-with <host> <code> --name work-pc
```

Now, any MCP tool or prompt can target `work-pc`:
```bash
# In Claude Code:
"On work-pc, open Terminal and check disk space"
```
