<div align="center">

# opendesk

**High-performance native computer-use framework for AI agents.**

Opendesk gives AI agents eyes and hands on your desktop just like a human would — screenshot capture, mouse, keyboard, semantic accessibility tree traversal, workflow automation, and peer-to-peer remote machine control.

**Windows · macOS · Linux**

[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](https://www.rust-lang.org/)
[![MCP](https://img.shields.io/badge/MCP-compatible-green)](https://modelcontextprotocol.io/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

</div>

---

## 🚀 Why Pure Rust?

OpenDesk has transitioned from its prototype Python implementation to a **100% native Rust architecture (`opendesk-rs`)**.

| Feature | Legacy Python Implementation | Native Rust Implementation (`opendesk-rs`) |
| :--- | :--- | :--- |
| **Deployment** | Python 3.10+, virtualenvs, pip/uv, OS C-extensions | **Single, static, standalone binaries** (`opendesk`, `opendesk-mcp`) |
| **Startup Latency** | ~500ms – 1.5s cold start | **< 10ms instantaneous cold start** |
| **Memory Footprint** | ~60MB – 120MB RSS | **~8MB – 18MB RSS** |
| **UI Automation** | Flat tag lists with loose control resolution | **Deep hierarchical accessibility trees (`xa11y`) with role & value attributes** |
| **Wire Protocol** | Base64-inflated JSON strings | **Zero-overhead native MessagePack binary (`bin`) with X25519 + ChaCha20-Poly1305** |
| **Input Reliability** | WinUI 3 buffer drops under raw keystroke loops | **Unicode SendInput typing (clipboard untouched) + keys pressed for the target's real keyboard layout** |

---

## 📦 Installation & Setup

### 1. Build & Install from Source

Prerequisites: [Rust and Cargo](https://rustup.rs/) (edition 2024 / Rust 1.80+).

```bash
git clone https://github.com/monatis/opendesk.git
cd opendesk

# Build and install release binaries to ~/.cargo/bin
cargo install --path opendesk-rs --force
```

This installs two binaries into your `PATH`:
- `opendesk`: CLI administration, local server, pairing, rendezvous relay, scheduler, and web UI.
- `opendesk-mcp`: Stdio Model Context Protocol (MCP) server for AI agent harnesses.

---

## 🤖 MCP Agent Setup

OpenDesk speaks the standard Model Context Protocol (MCP) and works out-of-the-box with **Claude Code**, **Claude Desktop**, **Cursor**, **Windsurf**, **Continue**, or any custom agent harness.

### Quick Setup for Claude Code
```bash
opendesk install
```
*(To uninstall: `opendesk uninstall`)*

### Claude Desktop & Cursor Configuration

Add to your `claude_desktop_config.json`:
- **Windows**: `%APPDATA%\Claude\claude_desktop_config.json`
- **macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Linux**: `~/.config/Claude/claude_desktop_config.json`

```json
{
  "mcpServers": {
    "opendesk": {
      "command": "opendesk-mcp"
    }
  }
}
```

---

## 🛠️ MCP Tools Reference

When connected via MCP, the agent receives a rich, deterministic set of desktop automation tools:

| Tool | Capabilities |
| :--- | :--- |
| **`screenshot`** | Captures display as PNG or lightweight JPEG with configurable quality (`format="jpeg"`, `quality=75`). Captures the whole virtual desktop (all monitors) or a region/zoom crop; mouse coordinates read off it are mapped back to the desktop automatically. |
| **`opendesk_view`** | Opens the interactive OpenDesk MCP App remote desktop viewport inline in supported hosts (**Goose**, **Claude Desktop**, custom web harnesses) for live observation, peer switching, and human-in-the-loop control. |
| **`ui`** | Inspects hierarchical accessibility trees (`action="get_tree"`), clicks elements by semantic role/title (`action="click"`), and types into active controls (`action="type"`). |
| **`keyboard`** | Types Unicode text without touching the clipboard (`action="type"`), presses keys and chords (`action="press"`/`"hotkey"`, e.g. `ctrl+s`, `ctrl+-`; characters use the target's real keyboard layout), holds keys (`action="hold"`), or runs timed sequences (`action="sequence"`). |
| **`opendesk_guard`** | Sets a foreground-window guard: `mouse`/`keyboard` input is refused unless the expected window (title/process) is in front, so keystrokes never land in the wrong app. Also available per call as `guard`. |
| **`opendesk_wait`** / **`opendesk_events`** | Wait until an element is attached/visible/enabled/focused/... instead of sleeping, and read an app's accessibility event log (focus, value, state, window, menu and live-region changes). |
| **`opendesk_exec`** / **`opendesk_job`** | Runs a shell command on the target. Timeouts kill the whole process tree and return the partial output. With `background: true` it returns a job id to poll for new output, or kill, with `opendesk_job`. |
| **`opendesk_file`** | Pushes and pulls files to and from a peer in verified chunks (SHA-256); the destination is only replaced once the whole file checks out. Also `opendesk push` / `opendesk pull`. |
| **`mouse`** | Pixel-accurate pointer movements (`action="move"`), clicks (`action="click"`), drags (`action="drag"`), and vertical/horizontal scrolling (`action="scroll"`). |
| **`app`** | Launches processes (`action="open"`), brings windows to foreground (`action="focus"`), closes windows/processes (`action="close"`), and enumerates running applications (`action="list"`). |
| **`clipboard`** | Reads (`action="read"`) and writes (`action="write"`) UTF-8 text from/to system clipboard. |
| **`audit`** | Shows (`action="show"`) or replays (`action="replay"`) the session audit log. |
| **`schedule`** | Stores named schedules (`action="add"`, `"list"`, `"remove"`). This build does not execute them. |
| **Remote Peer Routing** | `opendesk_peers`, `opendesk_use`, `opendesk_status`, and `opendesk_capabilities` (what a peer supports, plus its locale, keyboard layout, displays and clock offset). Every tool accepts an optional `peer="peer-name"` parameter to seamlessly target remote machines over encrypted RPC. |

---

## 🌐 Remote Machine Control (LAN & Internet)

OpenDesk allows an AI agent running on one machine (the *controller*) to drive any number of remote machines (*controlled peers*) with zero tool reconfiguration.

```
┌─────────────────┐       Encrypted Wire (X25519 + ChaCha20-Poly1305)       ┌─────────────────┐
│   Controller    │ ══════════════════════════════════════════════════════> │ Controlled Peer │
│ (Agent Harness) │   Pure Binary MessagePack (Display, Input, UI, Apps)    │ (opendesk serve)│
└─────────────────┘                                                         └─────────────────┘
```

### 1. Pairing (One-Time Mutual Trust Exchange)
On the machine you want to control:
```bash
opendesk pair
# Generates a secure, PBKDF2-stretched 6-digit one-time code and waits
```

On the controller machine:
```bash
# Discover peers on LAN via mDNS:
opendesk discover

# Pair using the displayed code:
opendesk pair-with <peer-ip-or-host> <code> --name work-pc
```

### 2. Serving Remote Control
On the controlled machine:
```bash
opendesk serve
```

### 3. Controlling Over MCP
The agent can target the remote peer at any time:
- In natural language: *"Open Notepad on work-pc and type hello"*
- Via explicit parameter: `screenshot(peer="work-pc")`
- Via active session focus: `opendesk_use(peer="work-pc")` (or back to local: `opendesk_use(peer="local")`)

### 4. Internet & NAT Traversal (Rendezvous Server)
When machines are on separate networks behind NATs or firewalls:
```bash
# 1. Run standalone rendezvous relay on a public VPS:
opendesk rendezvous --host 0.0.0.0 --port 8765 --token YOUR_TOKEN

# 2. Controlled peer establishes outbound tunnel:
opendesk serve --rendezvous ws://relay.example.com:8765 --rendezvous-token YOUR_TOKEN

# 3. Controller connects through relay:
opendesk connect work-pc --rendezvous ws://relay.example.com:8765 --rendezvous-token YOUR_TOKEN
```
*Note: All payloads through the relay remain strictly end-to-end encrypted with X25519 and ChaCha20-Poly1305. The relay server never has access to plaintext commands or screen captures.*

---

## 🖥️ Built-in Web UI & MCP Apps

### Standalone Web Dashboard
Launch OpenDesk's built-in web management dashboard:
```bash
opendesk app
```
Navigates to `http://127.0.0.1:8424`, providing:
- **Interactive Remote Desktop**: Live viewport with low-bandwidth, sub-second JPEG streaming (~850ms over relays, ~90–260 KB/frame vs 1.26 MB PNG) and instant 80ms interaction burst refresh.
- **Session Management**: Connected session inspection, peer switching, and live kick/evict capabilities.
- **Peer Registry**: Discovery, pairing, and trust store management.
- **Audit & Scheduler**: Real-time audit log streaming and background task scheduler management.

### MCP Apps (Inline Interactive Viewport in Goose, Claude Desktop)
OpenDesk supports the **MCP Apps (MCP UI) extension specification** (SEP-1865: `io.modelcontextprotocol/ui`). When using agent harnesses like **Goose**, **Claude Desktop**, or custom web frontends, the `opendesk_view` tool renders the complete interactive remote desktop viewport directly inside your chat conversation. The user can watch actions live without filling the LLM context window with images, or take over keyboard and mouse directly for human-in-the-loop tasks.

---

## ⚙️ Service Installation

Run OpenDesk as an unprivileged, user-scoped background daemon on boot:

```bash
# Install as user daemon (Windows Task Scheduler / macOS launchd / Linux systemd)
opendesk install-service

# Remove background daemon
opendesk uninstall-service
```

---

## 🏛️ Architecture & Protocols

```
┌─────────────────────────────────────────────────────────────┐
│  MCP Server Boundary        stdio JSON-RPC (opendesk-mcp)   │
├─────────────────────────────────────────────────────────────┤
│  Tools Surface              screenshot · ui · mouse · app · │
│                             keyboard · clipboard · schedule │
├─────────────────────────────────────────────────────────────┤
│  Computer Abstraction       LocalComputer · RemoteComputer  │
├─────────────────────────────────────────────────────────────┤
│  Native Backends            xa11y (UIA / AXUIElement)       │
│                             native screen capture & inputs  │
├─────────────────────────────────────────────────────────────┤
│  Encrypted Wire Protocol    Pure MessagePack (bin bytes)    │
│                             X25519 + ChaCha20-Poly1305 AEAD │
├─────────────────────────────────────────────────────────────┤
│  Transports                 Direct WebSocket · mDNS LAN     │
│                             Rendezvous Outbound Relay (NAT) │
└─────────────────────────────────────────────────────────────┘
```

---

## 📜 License & Acknowledgments

This project is licensed under the [MIT License](LICENSE).

Forked and modernized from the upstream [vitalops/opendesk](https://github.com/vitalops/opendesk) project created by Abhigith Neil Abraham, Fariz Rahman, and Fadil Rahman.
