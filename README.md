<div align="center">

# opendesk

**Give any AI agent eyes and hands on your desktop.**

Opendesk is a computer use framework that lets AI agents navigate your computer just like a human would — screenshots, mouse, keyboard, UI interaction, OCR, workflow recording, scheduling, and remote machine control.

**macOS · Linux · Windows**

[![Fork of vitalops/opendesk](https://img.shields.io/badge/fork%20of-vitalops%2Fopendesk-orange)](https://github.com/vitalops/opendesk)
[![Python 3.10+](https://img.shields.io/badge/python-3.10%2B-blue)](https://www.python.org/)
[![MCP 2.x](https://img.shields.io/badge/MCP-2.x%20compatible-green)](https://modelcontextprotocol.io/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

</div>

> [!NOTE]
> **OpenDesk (Enhanced Python Fork)**: This repository is a hardened, Python-first fork of the original [vitalops/opendesk](https://github.com/vitalops/opendesk) project.
> It introduces **Internet remote transport & NAT traversal** (rendezvous signaling and encrypted relay), **official MCP 2.x SDK compatibility**, **modern Windows (WinUI 3 / XAML Islands) UI automation**, **robust failsafe handling**, and **actionable leaf candidate ranking** for reliable autonomous agent execution.

---

<table>
<tr>
<td align="center" width="33%">

https://github.com/user-attachments/assets/5a6fab31-9f53-4ddb-9efb-17f0afe97844

<b>Single Machine Demo</b><br>Screenshot, click, type, navigate — all from natural language.
</td>
<td align="center" width="33%">

https://github.com/user-attachments/assets/629cf31b-12ab-4913-bc03-963f1cfbd682

<b>Control Multiple Machines</b><br>Drive remote desktops over encrypted WebSocket — same tools, same agent.
</td>
<td align="center" width="33%">

https://github.com/user-attachments/assets/659c9e30-e8f6-4a5a-ab81-0fa7ccaf8fb8

<b>UI Testing</b><br>Test a full web app — navigate, create, verify — with zero selectors.
</td>
</tr>
</table>

---

## 🌟 Fork Enhancements & Improvements

This fork focuses on turning OpenDesk into a production-grade, highly resilient computer-use framework for autonomous AI agents across local and distributed environments. Key additions and fixes include:

### 1. 🌐 Internet Remote Transport & NAT Traversal
- **Beyond LAN**: Extends OpenDesk's encrypted WebSocket transport from LAN-only to public Internet reachability.
- **NAT & Firewall Traversal**: Controlled endpoints establish outbound WebSocket tunnels to a rendezvous server, requiring **zero inbound port forwarding** or router reconfigurations.
- **Rendezvous & Relay Server**: Built-in, lightweight standalone rendezvous server (`opendesk rendezvous`) managing peer presence, token authentication, and relay frame exchange.
- **End-to-End Encryption Preserved**: The rendezvous relay handles strictly opaque encrypted packets. Session keys are negotiated directly between endpoints using **X25519** key exchange and authenticated with **ChaCha20-Poly1305 AEAD** — zero plaintext exposure to the relay.
- 📖 Read the full guide: [docs/remote/internet-transport.md](docs/remote/internet-transport.md).

### 2. 🐍 Pure Python Architecture
- Stripped legacy JavaScript dependencies and build configs to provide a clean, modern Python-first codebase.
- First-class support for fast modern package managers like [`uv`](https://github.com/astral-sh/uv) alongside `pip`.

### 3. 🔌 Official MCP 2.x & 1.x Compatibility
- **ServerRequestContext Alignment**: Resolved parameter inversion in MCP 2.x request handlers (`handle_call_tool(ctx, params)` vs `(params)`), eliminating runtime `AttributeError` exceptions when running on the latest official Python `mcp` SDK.
- **Dual-Mode Execution**: Transparently supports both low-level handler protocols and decorator-based MCP servers.
- **Structured Error Propagation**: Uncaught tool exceptions return structured MCP error responses (`is_error=True`) so AI agents receive clear diagnostics rather than silent dropouts.

### 4. 🪟 Windows 11 & WinUI 3 Modern Accessibility
- **Deep UIA Inspection**: Replaced superficial Win32 window enumeration with deep global UIAutomation traversal (`pywinauto` UIA backend), unlocking modern Windows 11 apps (WinUI 3 Notepad, Windows Terminal, Windows Settings, XAML Islands).
- **Accessible Name Resolution**: Extracts accessible element names from `element_info.name` and UIA control types, eliminating empty accessibility trees in modern controls.
- **Native Action Patterns**: Direct execution of native UIA action patterns (`invoke()`, `select()`, `toggle()`) in addition to bounding-box mouse clicks.
- **Hierarchical Menu Navigation**: Added `_windows_invoke_menu_path` to reliably traverse and click multi-level menu paths (e.g., `File -> Save`).

### 5. 🛡️ Agentic Robustness & Failsafe Hardening
- **PyAutoGUI Failsafe Elimination**: Disabled PyAutoGUI failsafe (`FAILSAFE = False`) during mouse/keyboard operations, preventing origin `(0, 0)` crashes in multi-monitor and headless agent environments.
- **OCR Tool Signature Alignment**: Aligned `ocr_image` signatures in `opendesk.computer.ocr` with thread-pool executor dispatches in `opendesk.tools.ocr`.
- **Unified Target Resolution**: Both `app` and `ui` tools accept process filenames (`notepad.exe`), base stems (`notepad`), and window titles (`Notepad`, `Untitled - Notepad`) interchangeably.
- **Scored Candidate Ranking**: Upgraded `ui(action='click', ...)` matching with heuristic candidate scoring (+100 exact, +60 prefix, +30 substring, +40 actionable leaf role, -40 container role), preventing agents from accidentally clicking outer window containers instead of internal buttons.

---

## Installation

Using [uv](https://github.com/astral-sh/uv) (recommended):
```bash
uv pip install 'opendesk[core,mcp]'
```

Or using standard `pip`:
```bash
pip install 'opendesk[core,mcp]'
```

> Requires Python 3.10+

---

## MCP install

opendesk works as an MCP server with any MCP-compatible client — Claude Code, Claude Desktop, Cursor, Windsurf, Continue, or any custom tool.

### Quick Setup (Claude Code)

```bash
opendesk install        # shortcut for Claude Code
```

### Other MCP clients (Claude Desktop, Cursor, Windsurf, Continue, custom)

Point your client at the `opendesk-mcp` binary:

```json
{
  "mcpServers": {
    "opendesk": { "command": "opendesk-mcp" }
  }
}
```

Once connected, try:

```
Take a screenshot of my screen
Click the Chrome icon
Open Spotify and play lo-fi beats
Show me the audit log
Replay everything from this session
```

---

## Python SDK usage

Use opendesk programmatically in your own agent or app:

```python
from opendesk import create_registry, allow_all_context

registry = create_registry()
ctx = allow_all_context()

result = await registry.get("screenshot").execute(ctx, ...)
```

---

## Architecture

opendesk is built in independently-importable layers:

```
┌──────────────────────────────────────────────────────────────┐
│  Integrations   MCP  ·  Claude Code  ·  OpenAI  ·  LangChain │
├──────────────────────────────────────────────────────────────┤
│  Tools          screenshot · mouse · keyboard · ui ·         │
│                 clipboard · ocr · learn · schedule · audit   │
├──────────────────────────────────────────────────────────────┤
│  Computer       LocalComputer  ·  RemoteComputer  (ABC)      │
├──────────────────────────────────────────────────────────────┤
│  Remote         server · client · discovery (mDNS)           │
├──────────────────────────────────────────────────────────────┤
│  Protocol       frames · codec (msgpack) · peer · transports │
│                 auth (X25519 + AEAD, pairing)                │
└──────────────────────────────────────────────────────────────┘
```

| Layer | What it does |
|-------|-------------|
| **Computer** | The capability surface of a computer (observe / act / subscribe). `LocalComputer` drives the local machine; `RemoteComputer` forwards every call over the wire to a paired peer. Tools and integrations target this ABC — they never know whether the machine is local or remote. |
| **Tools** | One class per capability, agent-friendly Pydantic schemas. Calls into the active `Computer` on the `ToolContext`. |
| **Integrations** | Thin adapters for MCP, Anthropic, OpenAI, LangChain — add one tool, get all four. |
| **Remote** | `opendesk serve` / `opendesk pair`, mDNS discovery, client helper. |
| **Protocol** | Five-frame wire protocol (msgpack binary, no base64 ever), WebSocket transport, mutual X25519 + ChaCha20-Poly1305 auth and encryption. |
| **Automation** | `learn` + `schedule` backed by pynput recording, JSON storage, APScheduler daemon. |

Full details → [docs/architecture.md](docs/architecture.md)

---

## Tools

| Tool | What it does |
|------|-------------|
| `screenshot` | Capture the screen with numbered boxes on every clickable element (Set-of-Marks) |
| `ui` | Click and type by element name — no coordinates needed |
| `mouse` | Pixel-level mouse control for anything `ui` can't reach |
| `keyboard` | Type text, press keys, send hotkeys |
| `app` | Open, close, and focus applications |
| `clipboard` | Read and write the system clipboard |
| `ocr` | Extract text from any region of the screen |
| `learn` | Record a workflow once, replay it anytime |
| `schedule` | Run any task or learned procedure on a timer |

Full reference → [docs/tools.md](docs/tools.md)

---

## Automation

Record a task once, replay it forever, or put it on a schedule.

**Record**
```
"Start recording task expense-form"
```
Perform the workflow yourself. The agent captures every click, keystroke, and screenshot.

**Replay**
```
"Stop recording"
"Replay expense-form"
```
The agent re-executes using the current screen state — no hardcoded coordinates.

**Schedule**
```
"Every morning at 9am, open my email in Chrome, take a screenshot, and summarize what's there"
"Schedule expense-form every friday at 5pm"
```
```bash
opendesk scheduler start
```

Supported timing: `every 30m` · `every 2h` · `every day at 09:00` · `every friday at 17:00` · raw cron

Full guide → [docs/automation.md](docs/automation.md)

---

## Remote computer use

Control another machine from your agent — same tools, same MCP server, the
`Computer` abstraction just lives on the other end of an encrypted WebSocket.

**On the machine being controlled** (one time):

```bash
pip install 'opendesk[core,remote]'
opendesk pair        # prints a 6-digit code, listens
```

**On the controller** (one time):

```bash
pip install 'opendesk[remote]'
opendesk discover                          # list opendesk peers on the LAN
opendesk pair-with <host> <code> --name mini
```

**After pairing**, the controlled machine runs the long-lived server:

```bash
opendesk serve            # accepts paired peers only
```

…and the controller drives it through the existing MCP server (Claude Code,
Claude Desktop, Cursor — anything that speaks MCP). The agent gets new admin
tools — `opendesk_peers`, `opendesk_use`, `opendesk_status` — and every
existing tool accepts an optional `peer:` argument:

```
screenshot                       → controls the local machine
screenshot peer=mini             → controls the paired remote
opendesk_use mini                → make mini the default for this session
screenshot                       → [on mini] ...
```

With exactly one paired peer the agent doesn't have to specify anything —
it becomes the implicit default. With multiple, the agent must pick
explicitly (no silent fallback).

**One controller at a time.** Pair as many machines as you like, but only
one drives the desktop at a time — a second peer trying to connect while
one is active gets a clean `BUSY` error. Same peer reconnecting bumps
the previous session (no waiting out a stale TCP). Two ways to free the
slot from the controlled machine:

- `opendesk disconnect` — **cooperative**. Server asks the controller to
  leave via a `session.evicted` PUSH; a cooperative client (the in-tree
  `RemoteComputer`) suppresses its auto-reconnect and raises
  `SessionEvicted`. Trust is preserved.
- `opendesk unpair <name>` — **enforced**. Revokes trust + closes the
  session; next reconnect fails authentication.

**Security model:** pairing exchanges long-lived X25519 keypairs via a 6-digit
code-authenticated handshake (PBKDF2-stretched, ~CPU-month to brute force).
Subsequent connections use mutual static-key authentication. Every frame is
ChaCha20-Poly1305 AEAD-encrypted with per-direction counters. No CA-signed
certificates required — the keys ARE the trust.

### Internet & NAT Remote Control (Rendezvous)

When controller and controlled machines are on different networks or behind firewalls/NATs:

**1. Run the rendezvous server** (on a server with a public IP or domain):
```bash
opendesk rendezvous --host 0.0.0.0 --port 8765 --token YOUR_SECRET_TOKEN
```

**2. On the machine being controlled** (maintains an outbound connection to the relay):
```bash
opendesk serve --rendezvous ws://rendezvous.example.com:8765 --rendezvous-token YOUR_SECRET_TOKEN
```

**3. On the controller**:
```bash
# Discover peers registered on the rendezvous server
opendesk discover --rendezvous ws://rendezvous.example.com:8765 --rendezvous-token YOUR_SECRET_TOKEN

# Pair with the remote machine
opendesk pair-with <target-pubkey> <code> --rendezvous ws://rendezvous.example.com:8765 --rendezvous-token YOUR_SECRET_TOKEN
```

All traffic remains end-to-end encrypted with X25519 and ChaCha20-Poly1305. The relay never sees unencrypted commands or screenshots.

Full guides → [docs/remote/index.md](docs/remote/index.md) · [docs/remote/internet-transport.md](docs/remote/internet-transport.md)

---

## Installation options

```bash
pip install opendesk                              # core framework only
pip install 'opendesk[core,mcp]'                  # + screen capture + MCP server (recommended)
pip install 'opendesk[core,mcp,remote]'           # + control another machine over LAN & Internet
pip install 'opendesk[core,mcp,learn]'            # + task recording and replay
pip install 'opendesk[core,mcp,learn,schedule]'   # + scheduled tasks
pip install 'opendesk[all]'                       # everything
```

---

## Platform support

| Feature | macOS | Linux | Windows |
|---------|:-----:|:-----:|:-------:|
| Screenshot | ✓ | ✓ | ✓ |
| Mouse & keyboard | ✓ | ✓ | ✓ |
| UI element access | AppleScript | AT-SPI2 | UI Automation |
| Clipboard | pbcopy/pbpaste | xclip/xsel | pyperclip |
| OCR | Vision / tesseract | tesseract | WinRT / tesseract |
| App control | `open -a` | `xdg-open` | `start` |
| Task recording | ✓ | ✓ | ✓ |
| Scheduled tasks | ✓ | ✓ | ✓ |
| Remote control (LAN & Internet / NAT) | ✓ | ✓ | ✓ |
| Rendezvous signaling & relay | ✓ | ✓ | ✓ |
| LAN discovery (mDNS) | ✓ | ✓ | ✓ |

---

## System permissions

### macOS
- **System Settings → Privacy & Security → Screen Recording** — enable for your terminal
- **System Settings → Privacy & Security → Accessibility** — enable for mouse/keyboard control

### Linux
```bash
sudo apt install xclip xdotool python3-atspi
```

### Windows
No extra permissions needed — opendesk uses Win32 APIs by default.

See [docs/permissions.md](docs/permissions.md) for full setup guide.

---

## Integrations

### Claude Code
```bash
opendesk install        # registers opendesk-mcp globally
opendesk uninstall      # removes the registration
```

### Claude Desktop

Add to your config file:
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

### Python API

```python
import asyncio
from opendesk import create_registry, allow_all_context

async def main():
    registry = create_registry()
    ctx = allow_all_context()

    result = await registry.get("screenshot").execute(
        ctx, registry.get("screenshot").Params(marks=True)
    )
    print(result.output)

asyncio.run(main())
```

Works with Anthropic SDK, OpenAI, and LangChain — see [docs/integrations.md](docs/integrations.md)

### On-device models (Ollama, LM Studio, vLLM, llama.cpp)

Any OpenAI-compatible local server works out of the box:

```python
from openai import OpenAI
from opendesk.integrations.openai_compat import OpenAIAdapter

client = OpenAI(base_url="http://localhost:11434/v1", api_key="ollama")
adapter = OpenAIAdapter()
result = await adapter.run_loop(client, model="qwen2.5:72b", messages=messages)
```

---

## Upstream & Citation

This project is a fork of the open-source [vitalops/opendesk](https://github.com/vitalops/opendesk) project created by Abhigith Neil Abraham, Fariz Rahman, and Fadil Rahman.

If you use OpenDesk in your research or project, please cite the upstream work:

```bibtex
@software{opendesk,
  author  = {Abraham, Abhigith Neil and Rahman, Fariz and Rahman, Fadil},
  title   = {opendesk: Open Desktop Automation Framework},
  year    = {2026},
  url     = {https://github.com/vitalops/opendesk},
  version = {0.2.0},
  license = {MIT}
}
```

A `CITATION.cff` is included — GitHub's "Cite this repository" button will pick it up automatically.

---

## License

MIT
