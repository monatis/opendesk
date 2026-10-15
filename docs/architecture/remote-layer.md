# Remote & Integrations Layers

## `remote/`

User-facing stitching for the LAN and Internet flows.

* `server.rs` — `OpendeskServer`. Accepts WebSocket connections (directly
  and/or through rendezvous relays), runs the pairing or auth handshake,
  exchanges HELLO, then handles `REQ` frames one at a time through
  `dispatch_call`. `run_pair` is the one-shot pairing mode behind
  `opendesk pair`.

  **Single-controller policy:** at most one peer holds an active session
  at any moment. Same-peer reconnect replaces the previous session (the
  common case after a Wi-Fi blip or controller crash). A *different*
  peer hitting an active server gets an error response with code `busy`
  ("server is busy: NAME is the active controller") and is disconnected.
* `audit.rs` — `AuditLog`. Append-only JSONL at
  `<home>/audit/YYYY-MM-DD.jsonl`. Records session lifecycle (opened,
  closed, rejected) plus every protocol method call with its outcome.
  `opendesk serve --no-audit` turns it off.
* `admin.rs` — local IPC: Unix socket `~/.opendesk/admin.sock` (mode
  0600), or on Windows localhost TCP on a port recorded in
  `~/.opendesk/admin.port`. Used by `opendesk sessions`,
  `opendesk disconnect`, and `opendesk unpair` / `opendesk peers remove`.
  No network exposure.
* `discovery.rs` — mDNS advertise / browse (`mdns-sd`). Service type
  `_opendesk._tcp.local.`.
* `rendezvous.rs` — the standalone relay server (`opendesk rendezvous`),
  the serve-side agent that registers with it, and the client side used to
  list and reach peers through it. Frames stay end-to-end encrypted.
* `client.rs` — `RemoteComputer`, `pair_with(...)`, and
  `connect(peer, rendezvous_url, rendezvous_token, home)`. With no peer it
  falls back to the persistent default peer (`opendesk peers default`).

## `service.rs`

Installs `opendesk serve` as a user-scoped service: a systemd user unit
(Linux), a launchd agent (macOS), or a Task Scheduler task that runs at
logon (Windows). Used by `opendesk install-service` /
`opendesk uninstall-service`.

## `automation/scheduler.rs`

`ScheduleStore` (the JSON file behind the `schedule` tool) and
`run_scheduler_daemon`, which only logs enabled schedules once a minute.

## `app/`

Backend for `opendesk app` (local web UI and REST API on
`http://127.0.0.1:8424`); the same UI is served as the MCP App behind
`opendesk_view`.

---

## `mcp/`

### `McpServer` (`server.rs`)

Single stdio MCP server (`opendesk-mcp`, or `opendesk mcp`). `list_tools`
returns the computer-use tools (each with an optional `peer` field) and the
admin tools (`opendesk_peers`, `opendesk_discover`, `opendesk_use`,
`opendesk_status`, `opendesk_describe`, `opendesk_capabilities`,
`opendesk_disconnect`, `opendesk_guard`, `opendesk_privacy`,
`opendesk_view`, `opendesk_app_api`).

`call_tool` resolves the target, applies the input guard to `mouse` /
`keyboard`, and runs the local or remote branch. Remote calls retry once on a
fresh connection after a connection error; cached connections idle for more
than 30 s are pinged before reuse.

### `McpSession`

Per-session state: the explicit default peer from `opendesk_use`, the
input guard, cached `RemoteComputer` connections, trusted peers, and
rendezvous settings. Resolution order for a call without `peer`:

1. Explicit default from `opendesk_use`
2. Persistent default peer (`opendesk peers default`)
3. `local`

### SDK adapters

The Python version's `claude_code.py`, `openai_compat.py` and
`langchain_compat.py` adapters do not exist in the Rust version; MCP is the
integration point.

---

Next: [Data Flow →](data-flow.md) — trace a call end-to-end, both locally and over the wire.
