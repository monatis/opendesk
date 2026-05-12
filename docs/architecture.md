# Architecture

## Overview

opendesk separates *what a computer can do* (the capability surface) from
*where that computer lives* (local vs. remote) and *how agents talk to it*
(via tools, MCP, etc.).  Each layer is independently importable.

```
┌──────────────────────────────────────────────────────────────┐
│  Integrations   MCP  ·  Claude Code  ·  OpenAI  ·  LangChain │
├──────────────────────────────────────────────────────────────┤
│  Tools          screenshot · mouse · keyboard · ui ·         │
│                 clipboard · ocr · app · learn · schedule     │
├──────────────────────────────────────────────────────────────┤
│  Computer       Computer ABC                                 │
│                   ├─ LocalComputer  (the machine we're on)   │
│                   └─ RemoteComputer (via the wire protocol)  │
│                 ComputerDispatcher (server-side router)      │
├──────────────────────────────────────────────────────────────┤
│  Remote         opendesk serve · pair · discover · connect   │
│                 mDNS advertisement & browsing                │
├──────────────────────────────────────────────────────────────┤
│  Protocol       frames · msgpack codec · peer (call-id mux)  │
│                 transports: WebSocket (TCP) — future: QUIC   │
│                 auth: X25519 + ChaCha20-Poly1305, pairing PSK│
└──────────────────────────────────────────────────────────────┘
```

Three load-bearing properties:

1. **Tools never know whether a Computer is local or remote.**  The same tool
   code runs against `LocalComputer` or `RemoteComputer`.
2. **Bytes are bytes.**  msgpack `bin` carries pixmaps, file contents, and
   process output natively — no base64 anywhere on the wire.
3. **Trust is keys, not certs.**  No CA-signed certificates required.  Both
   peers hold long-lived X25519 keypairs that authenticate each connection.

---

## Layer 1: `computer/`

### `Computer` ABC (`base.py`)

The capability surface of a computer.  Three kinds of operations:

* **Observe** (one-shot queries): `capture`, `cursor_position`, `displays`,
  `windows`, `focused_window`, `ui_tree`, `clipboard_read`, `processes`,
  `environment`, `read_file`, `list_dir`, `stat`, `notifications`.
* **Act** (one-shot state changes): `pointer`, `key`, `text`, `open_app`,
  `close_app`, `focus_app`, `focus_window`, `move_window`, `close_window`,
  `perform_ui_action`, `clipboard_write`, `write_file`, `delete`, `move`,
  `mkdir`, `shell`, `exec`, `lock_screen`.
* **Subscribe** (server-pushed streams): `subscribe_display`,
  `subscribe_input` — return async iterators.

Plus a sync `capabilities()` returning a `CapabilityManifest` so callers can
check support before attempting an operation.

Convenience helpers on top of the abstract primitives: `click`, `drag`,
`scroll`, `press`, `hotkey`, `type_text`, `clipboard_text` / `clipboard_set_text`.

### `LocalComputer` (`local.py`)

Concrete implementation for the current machine.  Wraps the existing screen
capture (mss), accessibility backends (AppleScript / AT-SPI2 / UI Automation),
input (pyautogui), and filesystem / process primitives.  All blocking I/O
runs in `asyncio.to_thread`.

### `RemoteComputer` (`remote.py`)

Implements the same `Computer` ABC by forwarding every call to a
:class:`Peer`.  Each abstract method serialises params, awaits
`peer.call(method, params)`, and validates the result back into a Pydantic
model.  Subscriptions return async iterators backed by `peer.stream(...)`.

`capabilities()` is synchronous (per the ABC) and served from the manifest
cached during the HELLO handshake — no round-trip.

### `ComputerDispatcher` (`dispatcher.py`)

Server-side router.  Implements the protocol's `Dispatcher` Protocol by
mapping each method name (`display.capture`, `input.pointer`, …) to the
matching `Computer` method, with Pydantic de/serialisation at the boundary.

### `types.py`

Pydantic value types exchanged across the boundary: `Point`, `Rect`,
`Pixmap` (with built-in pixel ↔ logical coordinate translation), `Display`,
`Window`, `Process`, `UIElement`, `PointerEvent`, `KeyEvent`,
`ClipboardContents`, `FileEntry`, `CompletedCommand`, `Environment`,
`Notification`, the `Capability` enum, and the `CapabilityManifest`.

### Auxiliary modules

* `capture.py` — mss-based screen capture. Wider-than-1920 displays are
  downscaled; the resulting `Pixmap` always carries the true logical screen
  dimensions so coordinates translate cleanly.
* `marks.py` — Set-of-Marks rendering helpers (`draw_som_marks`,
  `overlay_cursor`).  The element list is now produced by `Computer.ui_tree()`
  rather than by a separate accessibility query.
* `ocr.py` — `ocr_image(png_bytes)` runs the best available OCR backend
  (pytesseract → macOS Vision → WinRT OCR).  Decoupled from capture so it
  works on a `Pixmap` from any Computer, including a remote one.
* `sandbox.py` — per-session audit log used by tools that record actions.

---

## Layer 2: `tools/`

### `base.py`

```
ToolResult      — title, output, error, attachments, metadata
Attachment      — filename, content (bytes), media_type
ToolContext     — session_id, permission_handler, computer
Tool            — ABC: name, description, Params (Pydantic), execute()
```

The crucial field on `ToolContext` is **`computer: Computer`** — every tool
calls `ctx.computer.X(...)` instead of poking pyautogui / AppleScript
directly.  Swap in a `RemoteComputer` and the same tool runs against another
machine with zero code changes.

`ToolContext.check_permission(tool, argument, description)` calls the
injected handler before every action.  Raise `PermissionDeniedError` to
block.

### Tool files

Each file contains one `Tool` subclass.  None of them open subprocesses or
poke OS APIs directly — they always go through `ctx.computer`.

| File | Class | What it does |
|------|-------|-------------|
| `screenshot.py` | `ScreenshotTool` | `ctx.computer.capture()` + optional SoM marks via `ui_tree()` |
| `mouse.py` | `MouseTool` | `ctx.computer.click/drag/scroll` with image→logical coord translation |
| `keyboard.py` | `KeyboardTool` | `ctx.computer.text/press/hotkey` |
| `ui.py` | `UITool` | `ctx.computer.ui_tree()` + `perform_ui_action` (or bounds-center fallback) |
| `app.py` | `AppTool` | `ctx.computer.open_app/close_app/focus_app/list_apps` |
| `clipboard.py` | `ClipboardTool` | `ctx.computer.clipboard_read/clipboard_write` |
| `ocr.py` | `OCRTool` | `ctx.computer.capture()` → `ocr_image(pixmap.data)` |
| `automation.py` | `LearnTool`, `ScheduleTool` | Local session state; never remoted |
| `audit.py` | `AuditTool` | Local audit log; never remoted |

---

## Layer 3: `protocol/`

The transport-agnostic wire protocol.  Five frame types carry every byte
exchanged between peers:

| Frame | Direction | Purpose |
|---|---|---|
| `HELLO` | both | First frame each side sends. Carries protocol version, role, principal, auth proof, capability manifest. |
| `REQ` | caller → peer | Unary call (`stream=false`) or stream-starting call (`stream=true`). |
| `RES` | peer → caller | Response or one frame of a streaming response. Carries `result` or `error`. |
| `CANCEL` | either | Abort an in-flight call. |
| `PUSH` | either | Server-originated event not tied to a prior request. |

### `frames.py`

Pydantic models for the five frame types and `ErrorInfo` with a stable
`ErrorCode` enum (`capability_unsupported`, `permission_denied`,
`invalid_argument`, `not_found`, `timeout`, `cancelled`, `internal`,
`protocol`).

### `codec.py`

msgpack encode/decode.  Bytes round-trip as native msgpack `bin`; no base64
ever.  A `default` hook converts Python `set` and `Enum` instances to
list / value so Pydantic models with those types serialise cleanly.

### `connection.py`

The transport-shaped interface a `Peer` runs on.  `Connection` ABC with
`send(bytes)`, `recv() → bytes`, `aclose()`.  Includes `LoopbackConnection`
for hermetic in-process testing.

### `transports/websocket.py`

`WebSocketConnection`, `connect_websocket(url)`, `serve_websocket(handler, host, port)`.
Binary-only — text frames are rejected as a protocol violation.

### `peer.py`

`Peer` correlates call ids over a `Connection`.  Public API:

```python
peer = Peer(connection, role="client", dispatcher=optional)
await peer.hello(my_manifest)        # 1 RTT handshake
peer.start()                          # spawn recv loop
result = await peer.call("display.capture", {...})
async for frame in peer.stream("display.subscribe", {...}): ...
await peer.aclose()
```

Owns in-flight tables (unary, stream, inbound), routes cancellation in both
directions, maps wire error codes to Python exceptions (`cancelled` →
`CancelledError`, `permission_denied` → `PermissionDeniedError`, everything
else → `ProtocolError(code, message, details)`).

### `auth/`

* `identity.py` — `Identity` long-lived X25519 keypair persisted at
  `~/.opendesk/identity.key` (mode 0600, atomic write).
* `storage.py` — `TrustedPeers` JSON file with entries
  `{public_key, name, paired_at}`.
* `handshake.py` — two flavours:
  * `pair_server` / `pair_client` — 3-message PSK-authenticated handshake.
    PSK derived from the 6-digit code via PBKDF2-HMAC-SHA256 at 200 000
    iterations.  Both sides learn each other's static public key.
  * `auth_server` / `auth_client` — 2-message mutual-static-key handshake.
    Server rejects clients whose key isn't in `TrustedPeers`; client rejects
    servers that don't hold the expected static key.
* `encrypted.py` — `EncryptedConnection` wraps a Connection with
  ChaCha20-Poly1305 AEAD per frame.  Per-direction counter as the nonce;
  counter not transmitted (sender and receiver each maintain their own).
  Tamper or sync loss → `ConnectionClosed`.

---

## Layer 4: `remote/`

User-facing stitching for the LAN flow.

* `server.py` — `OpendeskServer`.  Accepts WebSocket connections, runs the
  right handshake (pair vs. auth), wraps each session in a `Peer` +
  `ComputerDispatcher`, tracks sessions in a `SessionRegistry`.
  `enable_pairing(code)` flips into one-shot pairing mode.
* `discovery.py` — `advertise(name, port, public_key)` and `discover(timeout)`
  via Zeroconf.  Service type `_opendesk._tcp.local.` with TXT records
  carrying the host's public key + fingerprint.
* `client.py` — `connect(target)` and `pair_with(host, port, code)`.
  Resolves a peer name → mDNS → WebSocket → `auth_client` → `RemoteComputer`.

---

## Layer 5: `integrations/`

### `mcp.py`

Single MCP server.  At list-tools time, augments Computer-use tool schemas
(`screenshot`, `mouse`, `keyboard`, `ui`, `app`, `clipboard`, `ocr`) with an
optional `peer` field; appends admin tools (`opendesk_peers`,
`opendesk_discover`, `opendesk_use`, `opendesk_status`,
`opendesk_capabilities`, `opendesk_disconnect`).

At call-tool time, strips `peer` from arguments, resolves to a `Computer`
via `MCPSession`, builds a fresh `ToolContext` with that Computer, and
dispatches.  Local-only tools (`learn`, `schedule`, `audit`) are passed
through unchanged.

### `MCPSession` (`mcp_session.py`)

Per-MCP-session state: explicit default peer, cache of open
`RemoteComputer` connections, the local `LocalComputer`.  Resolution order:

1. Per-call `peer:` argument
2. Explicit default from `opendesk_use`
3. Lone trusted peer (implicit default)
4. Multiple peers + no default → **error** (forces explicit choice)
5. No peers paired → local

### `claude_code.py`, `openai_compat.py`, `langchain_compat.py`

Adapters that present tools in the native format of each agent SDK.

---

## Data flow — local call

```
LLM
  │ tool_name + arguments
  ▼
Tool.parse_params  ← Pydantic
  ▼
ToolContext.check_permission
  ▼
Tool.execute(ctx, params)
  ▼
ctx.computer = LocalComputer
  ▼
mss / pyautogui / AppleScript / etc.
  ▼
ToolResult → Integration adapter → LLM
```

## Data flow — remote call

```
LLM
  │ tool_name + arguments (with optional peer:)
  ▼
MCPDispatcher: strips peer, resolves Computer
  ▼
Tool.execute(ctx, params)        ← ctx.computer = RemoteComputer
  ▼
peer.call("display.capture", ...)
  ▼
encode (msgpack) → AEAD encrypt → WebSocket binary frame → TCP
  ┌────────── over the wire ──────────┐
  ▼
TCP → WebSocket → AEAD decrypt → decode (msgpack)
  ▼
Peer dispatches → ComputerDispatcher
  ▼
LocalComputer on the remote machine
  ▼
result → encode → ... → back through wire → RemoteComputer
  ▼
ToolResult → Integration adapter → LLM
```

---

## Adding a custom tool

```python
from opendesk.tools.base import Tool, ToolContext, ToolResult
from opendesk.registry import create_registry
from pydantic import Field

class PingTool(Tool):
    name = "ping"
    description = "Check if a host is reachable from the active computer."

    class Params(Tool.Params):
        host: str = Field(description="Hostname or IP to ping.")
        count: int = Field(default=3, description="Number of pings.")

    async def execute(self, ctx: ToolContext, params: "PingTool.Params") -> ToolResult:
        result = await ctx.computer.exec(["ping", "-c", str(params.count), params.host])
        return ToolResult(
            title=f"Ping {params.host}",
            output=result.stdout_text(),
            error=result.returncode != 0,
        )

# Register and use
registry = create_registry()
registry.register(PingTool())
```

Because the tool calls `ctx.computer.exec(...)`, it runs on the local
machine when `ctx.computer = LocalComputer`, on a remote peer when
`ctx.computer = RemoteComputer` — same code, both paths.  And because the
schema comes from Pydantic, it automatically appears in MCP, Anthropic,
OpenAI, and LangChain adapters with no extra adapter code.
