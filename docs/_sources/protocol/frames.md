# Frames & Errors

## 7. Frame types

After the handshake, both sides exchange typed frames. Every frame is a
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
* `error` lets a side reject the session at the application level. Note that
  the Rust server rejects a second controller differently: it sends a RES
  frame with `id: 0` and `error.code = "busy"`
  (`"server is busy: NAME is the active controller"`) and closes the
  connection.

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
* `stream=true`: reserved for streaming calls (multiple RES frames with
  `end=false`, then a final one with `end=true`). The Rust server implements
  no streaming methods and answers every REQ with a single RES; requests on
  one session are handled one at a time.

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

Valid for both unary and streaming calls. The peer SHOULD respond with a
final RES carrying `error.code = "cancelled"`. Callers must tolerate
receiving the final RES before the CANCEL is processed. (The Rust server
currently only logs CANCEL; the call runs to completion.)

### PUSH

```
{
  "v":       1,
  "type":    "push",
  "topic":   "<string>",
  "payload": { ... }
}
```

Server-originated event not tied to any request. Receivers that don't
recognise a topic SHOULD ignore it.

**Known topics:**

| Topic | Payload | Meaning |
|---|---|---|
| `session.evicted` | `{"reason": "<string>"}` | Server asked the client to leave (cooperative disconnect) and is closing the connection. The Rust client ignores PUSH frames; its next call reconnects. |

---

## 8. Error codes

`ErrorCode` in `protocol/frames.rs` defines these stable strings:

| Code | Meaning |
|---|---|
| `capability_unsupported` | Method not supported by this platform or backend. |
| `permission_denied` | The call was refused. |
| `invalid_argument` | Malformed or out-of-range parameter. |
| `not_found` | Target resource (file, window, peer) doesn't exist. |
| `timeout` | Operation timed out on the server side. |
| `cancelled` | Call was cancelled (either side). |
| `internal` | Unclassified server-side error. |
| `protocol` | Wire-level violation (bad frame, wrong version, etc.). |
| `busy` | Server already has an active controller session. |

In practice the Rust server reports every failed method call with
`code = "error"` and the error text as `message` (including
`unknown method: NAME` and guard refusals such as `input refused: ...`); of
the codes above it currently only sends `busy`. Clients should treat any
non-null `error` as a failure and show its `message`.

---

Next: [Session Lifecycle & Method Namespace →](session.md)
