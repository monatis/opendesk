# Remote computer use — JavaScript / TypeScript SDK

Use an opendesk agent on one machine to control another over the LAN. Every
existing tool (`screenshot`, `mouse`, `keyboard`, `ui`, `app`, `clipboard`,
`ocr`) works the same — the `RemoteComputer` abstraction just lives on the
other end of an encrypted WebSocket.

```
┌─────────────────────┐    Wi-Fi / Ethernet    ┌─────────────────────┐
│  Controller         │   ws + msgpack + AEAD  │  Controlled machine │
│  (Claude Code, etc) │  ◄──────────────────►  │  opendesk-js serve  │
│  RemoteComputer     │                        │  ToolDispatcher     │
└─────────────────────┘                        └─────────────────────┘
```

> **Cross-SDK compatibility.** The JS and Python SDKs share the same wire
> protocol and the same `~/.opendesk/trusted-peers.json` format (snake_case
> keys). A machine paired with `opendesk pair` (Python) can be connected to
> from `connect()` (JS) and vice versa — no re-pairing needed.

---

## Mental model

| Term | Means |
|---|---|
| **Controlled machine** | The one being controlled. Runs `opendesk-js pair` once, then `opendesk-js serve` long-running. In protocol terms it's the **server** — it listens. |
| **Controller** | The machine running the agent (Claude Code, Claude Desktop, etc.). Runs `opendesk-js pair-with` once, then drives the controlled machine over MCP or the JS API. In protocol terms it's the **client** — it initiates. |
| **Pairing** | One-time exchange that establishes mutual trust via a 6-digit code shown on the controlled machine. After pairing both ends know each other's long-lived public keys and can reconnect without the code. |
| **Trusted-peers store** | `~/.opendesk/trusted-peers.json` on each side — the list of peers that machine has paired with. |
| **ToolDispatcher** | Server-side class that maps incoming `tool.*` RPC calls to local JS tool implementations and records each call to the audit log. |

---

## One-time setup

### On the controlled machine

```bash
npm install -g @vitalops/opendesk-sdk   # or npx for one-off use
opendesk-js pair
```

You'll see something like:

```
┌──────────────────────────────────────────────────────┐
│  opendesk pairing                                    │
│  port:        8423                                   │
│  fingerprint: 9c2f:1abc:b3d4:8870                    │
│                                                      │
│  pairing code:  428901                               │
│                                                      │
│  On the controller machine run:                      │
│    opendesk-js pair-with <host> 428901               │
└──────────────────────────────────────────────────────┘
```

Leave this terminal open while pairing. Once a controller successfully pairs
the process exits.

### On the controller

```bash
npx opendesk-js discover
```

Lists `_opendesk._tcp.local` services on the LAN:

```
NAME                      ADDR                    FINGERPRINT              DESCRIPTION
mac-mini                  192.168.1.42:8423       9c2f:1abc:b3d4:8870
```

Now pair (using the code shown on the controlled machine):

```bash
npx opendesk-js pair-with mac-mini.local 428901 --name mini
# Paired with mini (9c2f:1abc:b3d4:8870)
# Now reachable as: opendesk-js connect mini
```

Both machines now have each other's static public keys stored in
`~/.opendesk/trusted-peers.json`.

---

## Running

### Controlled machine

```bash
opendesk-js serve
```

Long-running daemon. Accepts paired peers only — refuses anyone whose static
key isn't in `trusted-peers.json`. Logs each connection / disconnection to
stderr. Writes one JSONL audit line per event to
`~/.opendesk/audit/<YYYY-MM-DD>.jsonl`.

Options:

```
opendesk-js serve [--port N] [--host H] [--no-mdns] [--approve {auto|console}]
```

- `--approve console` — prompts on stderr for every tool call instead of
  auto-approving. Requires a TTY; falls back to auto if stdin is not a TTY.
- `--no-mdns` — disable mDNS advertisement (useful in isolated environments).

### Controller

The agent uses opendesk through the existing MCP server — pointing Claude Code
at `opendesk-js mcp` just works. See *MCP integration* below.

For ad-hoc smoke tests:

```bash
npx opendesk-js connect mini
# Connected to mini  backend=local
# Screenshot: OK (received image data)
```

---

## MCP integration

Run `npx opendesk-js install` on the controller as before. The MCP server now
exposes:

**Computer-use tools** — `screenshot`, `mouse`, `keyboard`, `ui`, `app`,
`clipboard`, `ocr`. Each accepts an optional `peer:` argument.

**Admin tools** — for the agent to self-administer:

| Tool | Purpose |
|---|---|
| `opendesk_peers` | List `local` + every trusted peer, marked with `[default]` and `[active]`. |
| `opendesk_discover` | Browse the LAN for opendesk peers (paired and unpaired). |
| `opendesk_use <peer>` | Set the default peer for subsequent calls. `peer="local"` reverts. |
| `opendesk_status` | Show the effective default peer and open connections. |
| `opendesk_capabilities [peer]` | Capability manifest of a peer. |
| `opendesk_disconnect [peer]` | Close cached connection(s). |

### Default peer resolution

When a tool call omits `peer:`, opendesk picks one in this order:

1. The explicit default set via `opendesk_use`.
2. The lone trusted peer, if exactly one exists.
3. Otherwise — multiple peers paired, no default — **an error is returned**
   asking the agent to be explicit.
4. If no peers are paired at all, falls through to the local machine.

The ambiguous case (#3) deliberately errors instead of silently falling back
to local: a tool call that *meant* to land on a remote should never end up
on the host machine because the agent forgot.

### Agent example

```
> opendesk_peers
Available peers:
  local
  mini  [default (implicit)]  (9c2f:1abc:b3d4:8870)
  desk                        (3a48:7f0c:2211:0099)

> screenshot
ERROR: Multiple peers paired (mini, desk) and no default set.
Run `opendesk_use <name>` to choose one, or pass `peer:` on this call.

> opendesk_use mini
Default peer is now: mini

> screenshot
[on mini] Captured 1920×1200 screenshot...
```

### Why pairing isn't an MCP tool

The 6-digit code authenticates the pairing handshake. Codes should be typed
by a human looking at the controlled machine's screen, not flow through the
LLM channel where they might be logged, leaked, or replayed. `opendesk-js
pair` / `opendesk-js pair-with` are CLI-only by design.

---

## Audit log

`opendesk-js serve` writes one JSON line per event to
`~/.opendesk/audit/<YYYY-MM-DD>.jsonl` (directory `0700`, files `0600`).
Audit failures never crash the server.

Event types: `session.opened`, `session.closed`, `session.rejected`, `call`.

View the log:

```bash
opendesk-js audit                        # today's entries
opendesk-js audit --date 2026-05-13      # a specific date
opendesk-js audit --peer mini            # filter by peer name
opendesk-js audit --limit 50            # last 50 entries
opendesk-js audit --follow               # tail -f style, polls every 500 ms
```

Read programmatically:

```typescript
import { AuditLog } from "@vitalops/opendesk-sdk";

const log = new AuditLog();                    // defaults to ~/.opendesk/audit/
const entries = log.iterEntries();             // today
const older   = log.iterEntries("2026-05-13");

for (const e of entries) {
  console.log(e.ts, e.type, e.peer?.name, e.method, e.outcome);
}
```

---

## Trust and security

### What the threat model covers

- **Authentication.** Someone on your LAN cannot impersonate a paired
  controlled machine — the controller verifies the responder holds the X25519
  private key corresponding to the public key recorded during pairing.
  Symmetric for the controlled side.
- **Confidentiality.** Every protocol frame is ChaCha20-Poly1305 AEAD
  encrypted under per-direction session keys derived from a fresh
  Diffie-Hellman exchange in every connection.
- **Integrity.** Same AEAD provides integrity; tampering or replay causes the
  connection to close.
- **Forward secrecy.** Each connection uses fresh ephemeral keypairs;
  recording today's traffic and stealing a long-lived key tomorrow does not
  let an attacker decrypt the recording.

### What's not in v1

- **Internet-traversal.** The protocol is LAN-only for now. Cross-network
  needs STUN/TURN or a relay — phase 2.
- **CA-signed TLS.** Pairing uses out-of-band trust establishment (the code),
  not certificate authorities.
- **Multi-tenant accounts.** Trust is per-machine, single-user.

### Pairing-code strength

The code is 6 digits — 10⁶ possibilities. opendesk stretches the code via
PBKDF2-HMAC-SHA256 at 200 000 iterations before using it. At ~100 ms per
derivation on a modern CPU, exhausting the space takes ~a CPU-month. Pairing
only ever accepts one attempt and exits on success, so an online attacker has
one shot per `opendesk-js pair` invocation.

### Files written to disk

| Path | Mode | Contents |
|---|---|---|
| `~/.opendesk/identity.key` | `0600` | Raw 32-byte X25519 private key. Owner-only. Treat as a master secret. |
| `~/.opendesk/trusted-peers.json` | `0600` | `[{public_key, name, paired_at, description, description_override, last_host, last_port}, ...]` |
| `~/.opendesk/audit/<YYYY-MM-DD>.jsonl` | `0600` | Append-only audit log. One JSON object per line. |
| `~/.opendesk/description.txt` | `0600` | Broadcast description shown to controllers during discovery. |
| `~/.opendesk/default-peer` | `0600` | Persistent default peer name (written by `opendesk-js peers default`). |
| `~/.opendesk/admin.sock` | `0600` | Unix socket for CLI ↔ daemon IPC (Linux/macOS). Windows uses a localhost TCP port. |

---

## CLI reference

All commands accept `--home DIR` to override the identity / trusted-peers
location (default `~/.opendesk`).

### Controlled machine

```
opendesk-js pair        [--port N] [--code XXXXXX] [--timeout S] [--no-mdns]
opendesk-js serve       [--port N] [--host H] [--no-mdns] [--approve {auto|console}]
opendesk-js sessions                       # show the active controller (0 or 1)
opendesk-js disconnect                     # cooperative eviction of active controller
opendesk-js unpair NAME                    # revoke trust (+ disconnect if active)
opendesk-js describe [TEXT] [--clear]      # read/set/clear this machine's broadcast description
opendesk-js audit       [--date Y-M-D] [--peer NAME] [--limit N] [--follow]
```

### Controller

```
opendesk-js discover    [--timeout S]
opendesk-js pair-with   HOST CODE [--port N] [--name NAME]
opendesk-js connect     [PEER]
opendesk-js peers       [list]
opendesk-js peers       default [NAME | --clear]
opendesk-js peers       rename  NAME NEW-NAME
opendesk-js peers       remove  NAME
opendesk-js peers       describe NAME [TEXT] [--clear]
opendesk-js unpair      NAME
```

### Shared

```
opendesk-js install     [--scope {user|project}]
opendesk-js uninstall
opendesk-js mcp                            # run MCP server over stdio
```

---

## Concurrency: one controller at a time

`opendesk-js serve` enforces **single-controller** by design — at most one
controller machine drives the desktop at a time. Multiple controllers fighting
over a single mouse / keyboard / focused window is the failure mode this
policy prevents.

- **Different peer trying to connect while one is active** → rejected with a
  `BUSY` error.
- **Same peer reconnecting** (Wi-Fi blip, crash, network roam) → bumps the
  previous session cleanly.

To free the slot:

```bash
opendesk-js sessions           # show who's active
opendesk-js disconnect         # ask the active controller to leave (cooperative)
opendesk-js unpair NAME        # revoke trust entirely (enforced)
```

### disconnect vs unpair

Both commands talk to the running daemon over a local Unix socket /
named pipe (`~/.opendesk/admin.sock`) — only the user running `serve` can
issue them.

- **`opendesk-js disconnect`** is **cooperative**. The server sends a
  `session.evicted` PUSH frame and closes the connection. A cooperative client
  (`RemoteComputer`) sees the frame, suppresses auto-reconnect, and raises
  `SessionEvicted` on subsequent calls. Trust is preserved — the same
  controller can come back later.
- **`opendesk-js unpair NAME`** is **enforced**. The peer is removed from
  the trusted-peers store, then disconnected. Their next reconnect attempt
  fails authentication outright.

In short: `disconnect` says *"please leave, you can come back later"*;
`unpair` says *"you are not welcome here anymore."*

---

## Programmatic use

### Connect to a paired peer

```typescript
import { connect } from "@vitalops/opendesk-sdk";

const remote = await connect("mini");         // looks up ~/.opendesk/trusted-peers.json
const shot   = await remote.capture();
console.log(shot.width, shot.height);
await remote.close();
```

`RemoteComputer` exposes the same surface as the local computer —
`capture()`, `cursor()`, `pointer()`, `key()`, `windows()`, `clipboard()`,
`uiTree()`, `shell()`, and more. Auto-reconnect with exponential back-off is
on by default.

### Connect via DiscoveredPeer or explicit URL

```typescript
import { discover, connect } from "@vitalops/opendesk-sdk";

// mDNS browse
const [peer] = await discover(2000);          // { name, host, port, publicKey, fingerprint }
const remote = await connect(peer);

// Explicit URL (no trusted-peers store required)
const remote2 = await connect("ws://192.168.1.42:8423#<pubkey-hex>");
```

### Pair programmatically

```typescript
import { pairWith } from "@vitalops/opendesk-sdk";

const { remote, serverPubkey } = await pairWith(
  "192.168.1.42", 8423, "428901",
  { name: "mini" },
);
// remote is a connected RemoteComputer; serverPubkey is a Buffer
await remote.close();
```

### Run the server daemon from Node.js

```typescript
import {
  OpendeskServer, Identity, TrustedPeers,
  ToolDispatcher, AuditLog,
  createRegistry, allowAllContext,
} from "@vitalops/opendesk-sdk";

const identity = Identity.loadOrCreate();       // ~/.opendesk/identity.key
const trusted  = new TrustedPeers();            // ~/.opendesk/trusted-peers.json
const registry = createRegistry();
const ctx      = allowAllContext();
const audit    = new AuditLog();                // ~/.opendesk/audit/

const server = new OpendeskServer(identity, trusted, {
  audit,
  dispatcherFactory: ({ peerName, peerFingerprint, sessionId }) =>
    new ToolDispatcher({ registry, ctx, audit, peerName, peerFingerprint, sessionId }),
});

await server.start();
console.log("Listening on port", server.port);
```

### Identity and peer storage

```typescript
import { Identity, TrustedPeers, fingerprint } from "@vitalops/opendesk-sdk";

const identity = Identity.loadOrCreate();
console.log(fingerprint(identity.publicBytes));   // e.g. 9c2f:1abc:b3d4:8870

const peers = new TrustedPeers();
console.log(peers.list());
peers.setDefault("mini");
```

---

## Troubleshooting

**`opendesk-js discover` shows nothing.** The controlled machine must be
running `opendesk-js pair` or `opendesk-js serve` with `--no-mdns` *not* set.
mDNS is multicast UDP — it doesn't cross routers, only the same LAN segment.
Some Wi-Fi access points isolate clients ("AP isolation") — check the AP
settings.

**`opendesk-js pair-with` says "wrong_code".** Make sure the code on the
controlled machine matches what you typed. Codes change every `opendesk-js
pair` run.

**`opendesk-js serve` says "no trusted peers yet".** You haven't paired
anything. Run `opendesk-js pair` first; once one peer pairs, `serve` will
start accepting connections.

**The agent gets "Multiple peers paired … and no default set".** That's
deliberate. Run `opendesk_use <name>` from the agent (or pass `peer: <name>`
on each tool call) so opendesk knows where actions should land.

**Permissions on macOS.** `opendesk-js serve` needs Accessibility + Screen
Recording permission to drive mouse/keyboard and capture the screen — same as
the local CLI. Grant them to the `node` binary (or the terminal running it).

**Same machine testing (loopback).** When running controller and controlled on
the same machine, both share `~/.opendesk/trusted-peers.json`. Use `--home
/tmp/ctrl` on one side to keep the identity stores separate:

```bash
opendesk-js pair --home /tmp/ctrl --port 9000
# In another terminal:
opendesk-js pair-with 127.0.0.1 XXXXXX --home /tmp/ctrl --port 9000
```
