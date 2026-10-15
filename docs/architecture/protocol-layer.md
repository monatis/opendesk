# Protocol Layer

## `protocol/`

The wire protocol. Five frame types carry every message exchanged between
peers after the handshake:

| Frame | Direction | Purpose |
|---|---|---|
| `HELLO` | both | First frame each side sends. Carries protocol version, role, principal, auth data, capability map, optional error. |
| `REQ` | caller → peer | A call: `id`, `method`, `params`. |
| `RES` | peer → caller | `result` or `error` for a request id. |
| `CANCEL` | either | Abort an in-flight call (the server currently only logs it). |
| `PUSH` | either | Server-originated event not tied to a prior request (e.g. `session.evicted`). |

### `frames.rs`

serde types for the five frames (`Frame` is internally tagged by `type`) and
`ErrorInfo` with an `ErrorCode` enum (`capability_unsupported`,
`permission_denied`, `invalid_argument`, `not_found`, `timeout`, `cancelled`,
`internal`, `protocol`, `busy`). Frames are encoded with `rmp-serde`
(msgpack, named fields); dynamic params and results are `rmpv::Value`.

### `identity.rs`

`Identity` — long-lived X25519 keypair persisted at
`~/.opendesk/identity.key`, created on first use.

### `storage.rs`

`TrustedPeers` — `~/.opendesk/trusted-peers.json`: paired peers (public key,
name, description and local description override, last host/port, optional
rendezvous URL) and the persistent default peer.

### `handshake.rs`

* `pair_server` / `pair_client` — 3-message PSK-authenticated handshake.
  PSK derived from the 6-digit code via PBKDF2-HMAC-SHA256 at 200 000
  iterations. Both sides learn each other's static public key.
* `auth_server` / `auth_client` — 2-message mutual-static-key handshake.
  The server rejects clients whose key isn't trusted; the client rejects
  servers that don't hold the expected static key.

### `crypto.rs`

Session channel: ChaCha20-Poly1305 AEAD per frame, with separate send and
receive counters as the nonce (not transmitted). A failed decrypt ends the
session.

### Transport (`remote/transport.rs`)

`WebSocketTransport` — binary WebSocket messages for protocol frames (an
unexpected text message is an error). Text messages are used only for the
rendezvous relay's JSON control messages.

---

Next: [Remote & Integrations →](remote-layer.md)
