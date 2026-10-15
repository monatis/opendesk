# Concurrency: One Controller at a Time

`opendesk serve` enforces **single-controller** by design — at most one
controller machine drives the desktop at a time. Multiple controllers
fighting over a single mouse / keyboard / focused window is the failure
mode this policy prevents.

* **Different peer trying to connect while one is active** → rejected
  with a `busy` error ("server is busy: laptop is the active
  controller") and the connection is closed, so the agent knows to back
  off or ask the operator.
* **Same peer reconnecting** (Wi-Fi blip, controller crash, network
  roam) → bumps the previous session cleanly. No need to wait out the
  stale TCP connection.

You can still pair as many controllers as you like — they just take
turns. To free the slot:

```
opendesk sessions             # show who's active (0 or 1)
opendesk disconnect           # ask the active controller to leave (cooperative)
opendesk unpair NAME          # revoke trust entirely (enforced)
```

## Disconnect vs unpair — the trust model

Both commands talk to the running daemon over a local admin channel, not
the network: a Unix socket at `~/.opendesk/admin.sock` (mode 0600), or on
Windows a localhost TCP port recorded in `~/.opendesk/admin.port`. They
differ in *who has the last word*:

* **`opendesk disconnect`** is **cooperative**. The server sends a
  `session.evicted` PUSH frame and closes the connection. Trust is
  preserved — the same controller can reconnect (the Rust client does not
  act on the PUSH; its next call simply opens a new connection). A
  controller that should not come back needs `unpair`.
* **`opendesk unpair NAME`** is **enforced**. The peer is removed
  from the trusted-peers store, then disconnected (via the same admin
  IPC channel as `disconnect`). Their next reconnect attempt fails
  authentication outright.

In short: `disconnect` says *"please leave, you can come back later"*;
`unpair` says *"you are not welcome here anymore."*

---

Next: [Programmatic Use →](programmatic.md) — use the Rust crate, or MCP from any language.
