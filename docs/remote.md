# Remote computer use

Use an opendesk agent on one machine to control another over the LAN.  Every
existing tool (screenshot, mouse, keyboard, ui, app, clipboard, ocr) works
the same — the `Computer` abstraction just lives on the other end of an
encrypted WebSocket.

```
┌─────────────────────┐    Wi-Fi / Ethernet    ┌─────────────────────┐
│  Controller         │   ws + msgpack + AEAD  │  Controlled machine │
│  (Claude Code, etc) │  ◄──────────────────►  │  opendesk serve     │
│  RemoteComputer     │                        │  LocalComputer      │
└─────────────────────┘                        └─────────────────────┘
```

---

## Mental model

| Term | Means |
|---|---|
| **Controlled machine** | The one being controlled.  Runs `opendesk pair` once, then `opendesk serve` long-running.  In protocol terms it's the **server** — it listens. |
| **Controller** | The machine running the agent (Claude Code, Claude Desktop, etc.).  Runs `opendesk pair-with` once, then talks to the controlled machine over MCP / the Python API.  In protocol terms it's the **client** — it initiates. |
| **Pairing** | One-time exchange that establishes mutual trust via a 6-digit code shown on the controlled machine.  After pairing, both ends know each other's long-lived public keys and can reconnect without the code. |
| **Trusted-peers store** | `~/.opendesk/trusted-peers.json` on each side — the list of peers that machine has paired with. |

---

## One-time setup

### On the controlled machine

```bash
pip install 'opendesk[core,remote]'
opendesk pair
```

You'll see something like:

```
┌──────────────────────────────────────────────┐
│  opendesk pairing                            │
│  port:        8423                           │
│  fingerprint: 9c2f:1abc:b3d4:8870            │
│                                              │
│   pairing code:   428901                     │
│                                              │
│  Run on the controller:                      │
│    opendesk pair-with <host> 428901          │
└──────────────────────────────────────────────┘
```

Leave this terminal open while pairing.  Once a controller successfully
pairs, the process exits.

### On the controller

```bash
pip install 'opendesk[remote]'
opendesk discover
```

This lists `_opendesk._tcp.local` services on the LAN:

```
NAME                              ADDR                    FINGERPRINT
mac-mini-9c2f1a                   192.168.1.42:8423       9c2f:1abc:b3d4:8870
```

Now pair (using the code shown on the controlled machine):

```bash
opendesk pair-with mac-mini.local 428901 --name mini
```

Output:

```
✓ Paired with mini (9c2f:1abc:b3d4:8870)
  Now reachable as: opendesk connect mini
```

Both machines now have each other's static public keys stored.

---

## Running

### Controlled machine

```bash
opendesk serve
```

Long-running daemon.  Accepts paired peers only — refuses anyone whose
static key isn't in `trusted-peers.json`.  Logs each connection /
disconnection to stderr.  Survive reboots: see *Service install* below.

### Controller

The agent uses opendesk through the existing MCP server — pointing
Claude Code (or whatever) at `opendesk-mcp` just works.  See *MCP* below.

For ad-hoc smoke tests:

```bash
opendesk connect mini
# Connected to mini  backend=local/darwin
# Capabilities: ['apps.lifecycle', 'clipboard.read', 'clipboard.write', ...]

opendesk connect mini --screenshot ~/Desktop/mini-screen.png
```

---

## MCP integration

Run `opendesk install` on the controller as before.  The MCP server now
exposes:

**Computer-use tools** — `screenshot`, `mouse`, `keyboard`, `ui`, `app`,
`clipboard`, `ocr`.  Each accepts an optional `peer:` argument.

**Admin tools** — for the agent to self-administer:

| Tool | Purpose |
|---|---|
| `opendesk_peers` | List `local` + every trusted peer, marked with `[default (explicit/implicit)]` and `[active]`. |
| `opendesk_discover` | Browse the LAN for opendesk peers (paired and unpaired). |
| `opendesk_use <peer>` | Set the default peer for subsequent calls.  `peer="local"` reverts. |
| `opendesk_status` | Show the effective default peer and open connections. |
| `opendesk_capabilities [peer]` | Capability manifest of a peer. |
| `opendesk_disconnect [peer]` | Close cached connection(s). |

### Default peer resolution

When a tool call omits `peer:`, opendesk picks one in this order:

1. The explicit default set via `opendesk_use`.
2. The lone trusted peer, if exactly one exists.
3. Otherwise — if multiple peers are paired with no default —
   **an error is returned** asking the agent to be explicit.
4. If no peers are paired at all, falls through to the local machine.

The ambiguous case (#3) deliberately errors instead of silently falling
back to local: a tool call that *meant* to land on a remote should never
end up on the host machine because the agent forgot.

### Agent example

```
> opendesk_peers
Available peers:
  local
  mini  [default (implicit)]  (9c2f:1abc:b3d4:8870)
  desk                        (3a48:7f0c:2211:0099)

> screenshot
ERROR: Multiple peers paired (mini, desk) and no default set.
Run `opendesk_use <name>` to choose one, or pass `peer:` on this call
(use 'local' to target this machine).

> opendesk_use mini
Default peer is now: mini

> screenshot
[on mini] Captured 1920×1200 screenshot...
```

### Why pairing isn't an MCP tool

The 6-digit code authenticates the pairing handshake.  Codes should be
typed by a human looking at the controlled machine's screen, not flow
through the LLM channel where they might be logged, leaked, or replayed.
`opendesk pair` / `opendesk pair-with` are CLI-only by design.

---

## Trust and security

### What the threat model is

* **Authentication.**  Someone on your LAN cannot impersonate a paired
  controlled machine — the controller verifies the responder holds the
  X25519 private key corresponding to the public key recorded during
  pairing.  Symmetric for the controlled side.
* **Confidentiality.**  Every protocol frame is ChaCha20-Poly1305 AEAD
  encrypted under per-direction session keys derived from a fresh
  Diffie-Hellman exchange in every connection.
* **Integrity.**  Same AEAD provides integrity; tampering or replay causes
  the connection to close.
* **Forward secrecy.**  Each connection uses fresh ephemeral keypairs;
  recording today's traffic and stealing a long-lived key tomorrow does
  not let an attacker decrypt the recording.

### What's not in v1

* **Internet-traversal.**  The protocol is LAN-only for now.  Cross-network
  needs STUN/TURN or a relay — phase 2.
* **CA-signed TLS.**  Pairing uses out-of-band trust establishment (the
  code), not certificate authorities.  Optional `wss://` for defense in
  depth could be added.
* **Multi-tenant accounts.**  Trust is per-machine, single-user.

### Pairing-code strength

The code is 6 digits — 10⁶ possibilities.  Brute force matters because an
attacker who recorded the pairing exchange could try every code offline.
opendesk stretches the code via PBKDF2-HMAC-SHA256 at 200 000 iterations
before using it.  At ~25 ms per derivation, exhausting the space takes
~CPU-month.  Pairings only ever accept one attempt and exit on success,
so an online attacker has one shot per `opendesk pair` invocation.

### Files written to disk

| Path | Mode | Contents |
|---|---|---|
| `~/.opendesk/identity.key` | `0600` | Raw 32-byte X25519 private key.  Owner-only.  Treat as a master secret. |
| `~/.opendesk/trusted-peers.json` | `0600` | `[{public_key, name, paired_at}, ...]` |
| `~/.opendesk/pairing-code` | `0600` | Transient: present only while `opendesk pair` runs, deleted on success. |

---

## CLI reference

### Controlled machine

```
opendesk pair         [--port N] [--code XXXXXX] [--timeout S] [--no-mdns]
opendesk serve        [--port N] [--host H] [--no-mdns]
                      [--approve {auto,console}] [--no-audit] [--log-file PATH]
opendesk sessions                    # show the active controller (0 or 1)
opendesk disconnect                  # kick the active controller
opendesk unpair NAME                 # revoke trust + disconnect if active
opendesk check        [--open]       # macOS Accessibility / Screen Recording probe
opendesk audit        [--date Y-M-D] [--peer NAME] [--limit N] [--follow]
opendesk install-service / uninstall-service
```

### Controller

```
opendesk discover     [--timeout S]
opendesk pair-with    HOST CODE [--port N] [--name NAME]
opendesk connect      [PEER]         # PEER optional if a default is set
opendesk peers        [list | default [NAME|--clear] | rename NAME NEW | remove NAME]
```

All commands accept `--home DIR` to override the identity / trusted-peers
location (default `~/.opendesk`).

---

## Service install

Keep `opendesk serve` running across reboots:

```
opendesk install-service     # registers a systemd user unit (Linux),
                             # launchd agent (macOS), or scheduled task (Windows)
opendesk uninstall-service   # removes it
```

The service runs as your user, against `~/.opendesk`, on the default port.

---

## Concurrency: one controller at a time

`opendesk serve` enforces **single-controller** by design — at most one
controller machine drives the desktop at a time.  Multiple controllers
fighting over a single mouse / keyboard / focused window is the failure
mode this policy prevents.

* **Different peer trying to connect while one is active** → rejected
  with a clean `BUSY` error.  The client sees a `ProtocolError`
  ("server is busy: laptop is the active controller…") so the agent
  knows to back off or ask the operator.
* **Same peer reconnecting** (Wi-Fi blip, controller crash, network
  roam) → bumps the previous session cleanly.  No need to wait out the
  stale TCP connection.

You can still pair as many controllers as you like — they just take
turns.  To free the slot:

```
opendesk sessions             # show who's active (0 or 1)
opendesk disconnect           # ask the active controller to leave (cooperative)
opendesk unpair NAME          # revoke trust entirely (enforced)
```

### Disconnect vs unpair — the trust model

Both commands talk to the running daemon over a local Unix socket /
named pipe (`~/.opendesk/admin.sock`), not the network — only the user
running `serve` can use them.  They differ in *who has the last word*:

* **`opendesk disconnect`** is **cooperative**.  The server sends a
  ``session.evicted`` PUSH frame and closes the connection.  A
  cooperative client (the in-tree :class:`RemoteComputer`) sees the
  frame, suppresses its auto-reconnect, and raises
  :class:`SessionEvicted` on subsequent calls.  Trust is preserved —
  the same controller can come back when *it* decides to reconnect (by
  building a new RemoteComputer).  A hostile client could in principle
  ignore the PUSH and reconnect immediately; for that case, use
  ``unpair``.
* **`opendesk unpair NAME`** is **enforced**.  The peer is removed
  from the trusted-peers store, then disconnected (via the same admin
  IPC channel as `disconnect`).  Their next reconnect attempt fails
  authentication outright.

In short: `disconnect` says *"please leave, you can come back later"*;
`unpair` says *"you are not welcome here anymore."*

Read-only / observer-mode sessions where multiple controllers *could*
coexist (one driving, one watching) are a phase-2 design item; they'll
get a dedicated session kind, not a numeric knob.

---

## Programmatic use

The same trust-establishment + transport stack is available from Python:

```python
import asyncio
from opendesk.remote import connect

async def main():
    remote = await connect("mini")        # peer name from `opendesk peers list`
    try:
        pixmap = await remote.capture()
        print(pixmap.width, pixmap.height, len(pixmap.data))
    finally:
        await remote.aclose()

asyncio.run(main())
```

`remote` is a full `Computer` — drop it into any existing opendesk
`ToolContext` and every tool transparently targets the remote machine.

---

## Troubleshooting

**`opendesk discover` shows nothing.**  The controlled machine must be
running `opendesk pair` or `opendesk serve` with `--no-mdns` *not* set.
mDNS is multicast UDP — it doesn't cross routers, only the same LAN
segment.  Some Wi-Fi access points isolate clients from each other
("AP isolation") — check the AP settings.

**`opendesk pair-with` says "wrong_code".**  Make sure the code on the
controlled machine matches what you typed.  Codes change every `opendesk
pair` run.

**`opendesk serve` says "no trusted peers yet".**  You haven't paired
anything.  Run `opendesk pair` first; once one peer pairs, `serve` will
start accepting connections.

**The agent gets `Multiple peers paired (...) and no default set`.**
That's deliberate.  Run `opendesk_use <name>` from the agent (or pass
`peer: <name>` on each call) so opendesk knows where actions should land.

**Permissions on macOS.**  `opendesk serve` needs Accessibility + Screen
Recording permission to drive mouse/keyboard and capture the screen — same
as the local CLI.  Grant them to the Python / terminal binary running
`opendesk serve`.
