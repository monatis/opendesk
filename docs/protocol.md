# Protocol reference

opendesk's wire protocol carries every byte exchanged between a controller
(client) and a controlled machine (server).  This document covers the full
stack: transport, encoding, encryption, handshakes, frames, and mDNS
discovery.

---

## 1. Transport

All traffic runs over **binary WebSocket** (RFC 6455) on top of TCP.

* Default port: **8423** on the server (controlled machine).
* Text frames are rejected as a protocol violation.
* The transport is **reliable and ordered**, which the encryption layer depends
  on (see §3).
* Future: QUIC transport is planned for v2 (eliminates head-of-line blocking
  for concurrent streams).

---

## 2. Encoding

Every frame is **msgpack-encoded** (see `protocol/codec.py`).

Key conventions:
* `bytes` values round-trip as native msgpack `bin` — **never base64**.
  Pixmaps, file contents, and public keys are all raw bytes on the wire.
* Python `set` is encoded as a msgpack array.
* Python `Enum` is encoded as its `.value`.
* Unknown fields on received frames are ignored (forward-compatible reads).

---

## 3. Encryption

After a successful handshake (§5 or §6) the two peers share two 32-byte
session keys — one per direction — and all subsequent frames pass through
`EncryptedConnection` (`protocol/auth/encrypted.py`).

**Algorithm:** ChaCha20-Poly1305 AEAD (RFC 8439).

**Nonce:** A per-direction monotonic counter encoded as a 96-bit big-endian
integer.  The counter starts at 0, increments by 1 per frame, and is **not
transmitted** — both ends maintain their own copy.  Because the transport is
reliable and ordered, both counters stay in sync.

**Effect of tampering or desync:** AEAD verification fails → `ConnectionClosed`
is raised immediately on the receiving side.  There is no retry; the TCP
connection must be re-established and a new handshake run.

**Key lifetime:** Session keys are ephemeral — they exist for one TCP
connection.  Long-lived static keys (§4) never appear in frame payloads.

---

## 4. Identity and key material

Each machine holds a long-lived X25519 static keypair in its **identity**
(`protocol/auth/identity.py`):

| File | Mode | Contents |
|---|---|---|
| `~/.opendesk/identity.key` | `0600` | 32 raw bytes — the X25519 **private** key.  Never leaves disk. |

The **fingerprint** is derived from the public key by hex-encoding it and
grouping into four colon-separated 4-character chunks:
`abcd:ef01:2345:6789` (16 hex chars = first 8 bytes of the 32-byte key).

Trusted peers are stored in `~/.opendesk/trusted-peers.json` (mode `0600`,
directory mode `0700`).  Each entry:

```json
{
  "public_key": "<64-char hex>",
  "name": "mini",
  "paired_at": 1710000000.0,
  "description": "<cached from peer's mDNS/HELLO>",
  "description_override": "<user-set local label>",
  "last_host": "192.168.1.42",
  "last_port": 8423
}
```

`description_override` wins over `description` when non-empty (exposed as
`effective_description`).  `last_host`/`last_port` allow reconnecting without
an mDNS round-trip — essential in WSL2 or AP-isolation environments.

---

## 5. Pairing handshake (first contact)

Pairing establishes mutual trust between two machines that have never spoken
before.  The shared secret is a **6-digit numeric code** displayed on the
controlled machine and typed by the operator on the controller.

### PSK derivation

```
PSK = PBKDF2-HMAC-SHA256(
    password = code.encode("utf-8"),
    salt     = b"opendesk-psk-v1",
    iterations = 200 000,
    dklen    = 32,
)
```

200 000 iterations costs ~100 ms on a modern CPU, making offline brute-force
of the 10⁶-entry code space take roughly a CPU-month.

### Wire messages (3-message exchange)

```
1. C → S:  {"v": 1, "kind": "pair_hello", "e": <e_c_pub 32 B>}

2. S → C:  {"v": 1, "kind": "pair_offer",
                    "e": <e_s_pub 32 B>,
                    "ct": AEAD(k1, nonce=0, plaintext=s_s_pub)}

3. C → S:  {"v": 1, "kind": "pair_finish",
                    "ct": AEAD(k2, nonce=0, plaintext=c_s_pub)}
```

All byte fields are raw msgpack `bin`.

### Key derivation inside the handshake

```
k1 = HKDF-SHA256(salt=PSK,       ikm=DH(e_s, e_c),  info="opendesk-pair-k1")
k2 = HKDF-SHA256(salt=PSK||ct1,  ikm=DH(e_c, e_s),  info="opendesk-pair-k2")
```

`ct1` in the salt of k2 binds the session to the server's encrypted static
key, preventing transcript substitution attacks.

### Session key derivation (pairing)

After message 3:

```
transcript = SHA-256(e_s_pub || e_c_pub || ct1 || ct2)

dh_chain = [
    DH(e_s, e_c),    # ephemeral × ephemeral
    DH(s_s, e_c),    # server static × client ephemeral
    DH(e_s, s_c),    # server ephemeral × client static
]

material = concat(dh_chain) || PSK

64 bytes = HKDF-SHA256(salt=transcript, ikm=material,
                        info="opendesk-session-keys", length=64)

c2s_key = material[:32]   # client→server direction
s2c_key = material[32:]   # server→client direction
```

Each side derives both keys; their send key is the other side's receive key.

### Outcome

Both peers now hold each other's static public key (`s_s_pub` / `s_c_pub`),
which they persist in their `trusted-peers.json`.  The session continues
immediately on the same connection, encrypted with the derived session keys.

---

## 6. Auth handshake (reconnect)

After pairing, subsequent connections use a **2-message static-key handshake**
— no code required.

### Wire messages

```
1. C → S:  {"v": 1, "kind": "auth_hello",
                    "e": <e_c_pub 32 B>,
                    "s": <s_c_pub 32 B>}

2. S → C:  {"v": 1, "kind": "auth_offer",
                    "e": <e_s_pub 32 B>,
                    "ct": AEAD(k, nonce=0, plaintext=b"ok")}
```

If the server does not recognise the client's static key (`s_c_pub`), it
still responds with message 2 but with `ct = b""` (zero bytes), then closes
— deliberately ambiguous to avoid leaking whether the key was unknown vs.
the DH was bad.

### Key derivation (auth)

```
transcript = SHA-256(e_s_pub || e_c_pub || s_c_pub)

k = HKDF-SHA256(salt=transcript,
                ikm=DH(e_s, e_c) || DH(s_s, e_c),
                info="opendesk-auth-k")
```

The `b"ok"` payload encrypted under `k` gives the client proof that the server
holds the private key for `s_s_pub` (recorded at pairing time).

### Session key derivation (auth)

```
dh_chain = [
    DH(e_s, e_c),   # server ephemeral × client ephemeral
    DH(s_s, e_c),   # server static    × client ephemeral
    DH(e_s, s_c),   # server ephemeral × client static
    DH(s_s, s_c),   # server static    × client static
]

64 bytes = HKDF-SHA256(salt=transcript, ikm=concat(dh_chain),
                        info="opendesk-session-keys", length=64)
```

No PSK is mixed in for auth (only for pairing).

### Auth failure codes

| `reason` | Meaning |
|---|---|
| `wrong_code` | AEAD decryption failed — PSK mismatch (wrong pairing code). |
| `untrusted_peer` | Client's static key not in server's `trusted-peers.json`. |
| `unexpected_peer` | Server's auth proof didn't verify — server holds a different key than expected. |
| `protocol` | Malformed message, wrong version, or wrong `kind`. |

---

## 7. Frame types

After the handshake, both sides exchange typed frames.  Every frame is a
msgpack dict with at minimum `{"v": 1, "type": "<name>", ...}`.

### HELLO

```
{
  "v":            1,
  "type":         "hello",
  "role":         "client" | "server",
  "principal":    "<friendly name, may be empty>",
  "auth":         {},          // reserved for future use
  "capabilities": { ... },    // CapabilityManifest dict
  "error":        null | {"code": "...", "message": "...", "details": {}}
}
```

* **First frame** sent by each side after the encrypted connection is
  established, before any REQ/RES exchange.
* `capabilities` carries the server's `CapabilityManifest` (supported methods,
  platform, backend info) so the client can check before calling.
* A server can **reject** the connection at the application level by setting
  `error` — e.g. `{"code": "busy", "message": "laptop is the active controller"}`.
  The client's `Peer.hello()` raises `ProtocolError` on a non-null error field.

### REQ

```
{
  "v":      1,
  "type":   "req",
  "id":     <monotonic integer, unique per Peer>,
  "method": "<namespace>.<name>",
  "stream": false | true,
  "params": { ... }
}
```

* `stream=false` (default): unary call, expects exactly one RES.
* `stream=true`: streaming call, expects multiple RES frames with `end=false`
  followed by a final RES with `end=true`.

### RES

```
{
  "v":      1,
  "type":   "res",
  "id":     <matches the REQ id>,
  "seq":    <0-based frame counter for streams>,
  "end":    true | false,
  "result": { ... } | null,
  "error":  null | {"code": "...", "message": "...", "details": {}}
}
```

* Unary: exactly one frame, `end=true`, `seq=0`.
* Streaming: N frames with `end=false`, then one with `end=true` (may have
  `result=null` if the stream ended cleanly).
* `result` and `error` are mutually exclusive; both null means empty success.

### CANCEL

```
{
  "v":      1,
  "type":   "cancel",
  "id":     <id of the call to abort>,
  "reason": "<optional human-readable string>"
}
```

Valid for both unary and streaming calls.  The peer SHOULD respond with a
final RES carrying `error.code = "cancelled"`.  Callers must tolerate
receiving the final RES before the CANCEL is processed.

### PUSH

```
{
  "v":       1,
  "type":    "push",
  "topic":   "<string>",
  "payload": { ... }
}
```

Server-originated event not tied to any request.  Receivers that don't
recognise a topic SHOULD ignore it.

**Known topics:**

| Topic | Payload | Meaning |
|---|---|---|
| `session.evicted` | `{"reason": "<string>"}` | Server asked the client to leave (cooperative disconnect). A well-behaved client suppresses auto-reconnect and raises `SessionEvicted` on subsequent calls. |

---

## 8. Error codes

The closed set of `ErrorCode` values (stable strings):

| Code | Python exception | Meaning |
|---|---|---|
| `capability_unsupported` | `CapabilityUnsupported` | Method not supported by this platform or backend. |
| `permission_denied` | `PermissionDeniedError` | Server-side policy gate rejected the call. |
| `invalid_argument` | `ProtocolError` | Malformed or out-of-range parameter. |
| `not_found` | `ProtocolError` | Target resource (file, window, peer) doesn't exist. |
| `timeout` | `ProtocolError` | Operation timed out on the server side. |
| `cancelled` | `asyncio.CancelledError` | Call was cancelled (either side). |
| `internal` | `ProtocolError` | Unclassified server-side error. |
| `protocol` | `ProtocolError` | Wire-level violation (bad frame, wrong version, etc.). |
| `busy` | `ProtocolError` | Server already has an active controller session. |

---

## 9. Session lifecycle

```
TCP connect
  │
  ▼
Handshake (pair or auth) → EncryptedConnection
  │
  ▼
HELLO exchange (one frame each direction, simultaneous)
  │  client.hello(my_manifest) ──► server.hello(my_manifest)
  │  client ◄── server HELLO (capabilities + optional error)
  │
  ▼  [server HELLO error → ProtocolError, connection closes]
  │
  ▼
peer.start()  ← spawns background recv loop
  │
  ├── outbound: peer.call(method, params) → result dict
  │                or peer.stream(method, params) → AsyncIterator
  │
  ├── inbound:  Dispatcher.call / Dispatcher.stream (server side)
  │
  ├── push:     peer.push(topic, payload)  /  peer.on_push(handler)
  │
  ▼
peer.aclose() or connection drop
  └── all in-flight calls fail with ConnectionClosed
```

**Single-controller invariant (server side):** at most one session exists at a
time.  The `accept_lock` in `OpendeskServer._handle_connection` serialises the
"check-existing + register-new" step:

* Same peer reconnecting → old session evicted (`session.evicted` PUSH sent),
  new session registered.
* Different peer while one is active → HELLO sent with `error.code = "busy"`,
  connection closed.

---

## 10. Method namespace

Method names are `<namespace>.<verb>` strings.  The full set dispatched by
`ComputerDispatcher`:

### Observation methods (auto-approved by ConsolePolicy)

| Method | Description |
|---|---|
| `display.capture` | Screenshot → Pixmap |
| `display.displays` | List monitors |
| `display.cursor_position` | Current pointer position |
| `display.subscribe` | Streaming display frames |
| `windows.list` | All windows |
| `windows.focused` | Focused window |
| `ui.tree` | Accessibility element tree |
| `clipboard.read` | Read clipboard contents |
| `fs.read` | Read file bytes |
| `fs.list` | List directory |
| `fs.stat` | File metadata |
| `process.list` | Running processes |
| `apps.list` | Installed / running applications |
| `notifications.list` | Recent notifications |
| `input.subscribe` | Streaming input events |
| `system.capabilities` | Capability manifest |
| `system.environment` | Environment variables + platform info |

### Action methods (require policy approval)

| Method | Description |
|---|---|
| `input.pointer` | Mouse move / click / drag / scroll |
| `input.key` | Key press / release / chord |
| `input.text` | Type a string |
| `apps.open` | Launch application |
| `apps.close` | Quit application |
| `apps.focus` | Bring application to front |
| `windows.focus` | Focus a specific window |
| `windows.move` | Reposition / resize a window |
| `windows.close` | Close a window |
| `ui.perform` | Perform accessibility action on element |
| `clipboard.write` | Write clipboard contents |
| `fs.write` | Write file bytes |
| `fs.delete` | Delete file or directory |
| `fs.move` | Move / rename file |
| `fs.mkdir` | Create directory |
| `process.shell` | Run shell command |
| `process.exec` | Run process directly (no shell) |
| `system.lock` | Lock the screen |

---

## 11. mDNS discovery

opendesk advertises itself on the LAN via Zeroconf / Bonjour.

**Service type:** `_opendesk._tcp.local.`

**TXT record properties:**

| Key | Type | Value |
|---|---|---|
| `v` | string | Protocol version, currently `"1"`. |
| `pk` | bytes (binary) | 32-byte X25519 static public key. |
| `fp` | string | Colon-separated fingerprint for human display. |
| `desc` | string | Truncated self-description (max 120 UTF-8 bytes, cut at a character boundary). |

`desc` is a short version of whatever the operator set via `opendesk describe`.
The full description flows over the HELLO frame after authentication.

**Discovery:** `discover(timeout=2.0)` browses for `timeout` seconds and
returns a list of `DiscoveredPeer` objects.  Peers without a valid `pk` TXT
record are silently skipped.

**Limitations:**
* mDNS is multicast UDP — it does not cross IP routers.
* Some Wi-Fi access points enable client isolation, blocking mDNS between
  devices on the same SSID.
* WSL2 (NAT mode) blocks mDNS in both directions.  opendesk caches the last
  known `(host, port)` of each peer in `trusted-peers.json` and tries that
  first on reconnect, bypassing the mDNS limitation.  Alternatively, enable
  WSL2 mirrored networking mode (`networkingMode=mirrored` in `~/.wslconfig`).

---

## 12. Capability manifest

The server sends its `CapabilityManifest` in the HELLO frame's `capabilities`
field.  Fields include:

* `capabilities` — list of `Capability` enum values the server supports
  (e.g. `"display.capture"`, `"input.pointer"`, …).
* `platform` — `"darwin"` | `"linux"` | `"windows"`.
* `backend` — accessibility backend name (`"appscript"`, `"atspi"`, `"uia"`, …).
* `description` — the server's self-description string (full, untruncated).

The client stores this manifest locally (no round-trip per call) and uses it
to raise `CapabilityUnsupported` before sending a REQ for a method the server
doesn't support.

---

## 13. Admin IPC

`OpendeskServer` exposes a local admin socket for CLI management commands.
It is **never reachable over the network**.

| Platform | Path |
|---|---|
| Linux / macOS | `~/.opendesk/admin.sock` (Unix domain socket) |
| Windows | `\\.\pipe\opendesk-admin` (named pipe) |

Used by: `opendesk sessions`, `opendesk disconnect`, `opendesk unpair`, and
the web UI's disconnect / unpair API endpoints.

---

## 14. Cryptographic summary

| Primitive | Usage |
|---|---|
| X25519 | Static identity keypairs; ephemeral keypairs per handshake |
| PBKDF2-HMAC-SHA256 (200 000 iter) | PSK stretching from 6-digit pairing code |
| HKDF-SHA256 | Key derivation in both handshakes and session key material |
| ChaCha20-Poly1305 | Per-frame AEAD encryption of all session traffic |
| SHA-256 | Transcript hashing to bind session keys to the exchange |
| msgpack | Serialisation of handshake messages and protocol frames |
