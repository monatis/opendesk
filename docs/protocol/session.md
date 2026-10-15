# Session Lifecycle & Method Namespace

## 9. Session lifecycle

```
TCP connect (direct, or relayed through a rendezvous server)
  │
  ▼
WebSocket upgrade
  │
  ▼
Handshake (pair or auth) → encrypted session channel
  │
  ▼
Single-controller check (server side)
  │  different peer active → RES {id: 0, error.code: "busy"}, connection closes
  │  same peer reconnecting → previous session replaced
  ▼
HELLO exchange (capabilities)
  │
  ▼
Request loop (server handles one REQ at a time)
  │  REQ → dispatch_call → RES          (each call written to the audit log)
  │  admin disconnect → PUSH "session.evicted", connection closes
  ▼
Connection drop or close
```

Implementation: `handle_inbound_connection`, `run_session_loop` and `dispatch_call` in
`opendesk-rs/src/remote/server.rs`.

---

## 10. Method namespace

Method names are `<namespace>.<verb>` strings. The full set handled by
`dispatch_call` on the controlled machine:

| Method | Description |
|---|---|
| `display.capture` | Screenshot. Params: `format` (`jpeg` default / `png`), `quality` (75), `max_dim`, `region` `{x, y, width, height}`. Returns `{data (bin), format, width, height}`. |
| `input.pointer` | Mouse `event.action`: `move`, `click`, `scroll`, `down`, `up`; `event.point {x, y}`, `event.button`, `event.dy`. `space: "image"` maps the point from the last screenshot onto the desktop. |
| `input.text` | Type a string (`text`). |
| `input.key` | Press a key or chord (`key`, e.g. `ctrl+-`); `action` `down` / `up` for hold/release. |
| `input.sequence` | Run `steps` (`key` / `text` / `hotkey` / `keys`, `delay_ms`) atomically. |
| `process.exec`, `process.shell` | Run `command` (with optional `shell`) or `argv`, with `timeout_secs`. Returns `{stdout, stderr, exit_code, success, timed_out}`; on timeout the process tree is killed and partial output returned. |
| `apps.open` / `apps.close` / `apps.focus` | By `name`. |
| `apps.list`, `windows.list` | Running applications / windows. |
| `ui.tree` | Accessibility tree of `app`. |
| `ui.action` | Click an element (`element.name` or `selector`) in `app`. |
| `ui.value` | Read an element's value. |
| `clipboard.read` / `clipboard.write` | Clipboard text. |
| `system.capabilities` | Capability map. |
| `system.environment` | Platform information. |
| `system.privacy` / `system.privacy_status` | Physical input lock and blackout curtain (used by `opendesk_privacy`). |

Any other method name fails with `unknown method: NAME`.

**Input guard:** any `input.*` call may carry `guard: {title, process}`. The
server checks its own foreground window immediately before sending the input
and fails the call with `input refused: ...` if it does not match.

There is no server-side approval policy in the Rust version: a paired,
authenticated controller can call every method. The older `fs.*`,
`windows.move` / `focus` / `close`, `notifications.list`, `*.subscribe`,
`ui.perform` and `system.lock` methods are not implemented.

---

Last section: [Discovery, Admin IPC & Cryptographic Summary →](discovery.md)
