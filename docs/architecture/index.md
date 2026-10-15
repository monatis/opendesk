# Architecture

opendesk is a single Rust crate, `opendesk-rs`, with two binaries: `opendesk`
(CLI, `serve`, pairing, relay, web UI) and `opendesk-mcp` (stdio MCP server).
It separates *what a computer can do* (the `computer` module) from *where that
computer lives* (local vs. remote) and *how agents talk to it* (MCP).

```
┌──────────────────────────────────────────────────────────────┐
│  MCP server     mcp/server.rs — stdio JSON-RPC, tool list,   │
│                 peer routing, input guard, MCP App viewport  │
├──────────────────────────────────────────────────────────────┤
│  Tools          screenshot · mouse · keyboard · ui · app ·   │
│                 clipboard · opendesk_exec · audit · learn ·  │
│                 schedule · opendesk_* admin tools            │
├──────────────────────────────────────────────────────────────┤
│  Computer       LocalComputer   (the machine we're on)       │
│                 RemoteComputer  (via the wire protocol)      │
│                 dispatch_call   (server-side method router)  │
├──────────────────────────────────────────────────────────────┤
│  Remote         opendesk serve · pair · discover · connect   │
│                 mDNS · rendezvous relay · admin IPC · audit  │
├──────────────────────────────────────────────────────────────┤
│  Protocol       frames · msgpack · WebSocket transport       │
│                 X25519 + ChaCha20-Poly1305, PBKDF2 pairing   │
└──────────────────────────────────────────────────────────────┘
```

Three load-bearing properties:

1. **Tools have a local and a remote path with the same behaviour.** The MCP
   server calls `LocalComputer` for `local` and `RemoteComputer` for a paired
   peer; on the peer, `dispatch_call` maps each protocol method back onto its
   own `LocalComputer`.
2. **Bytes are bytes.** msgpack `bin` carries screenshots natively — no base64
   on the wire (MCP image results are base64, as MCP requires).
3. **Trust is keys, not certs.** No CA-signed certificates required. Both
   peers hold long-lived X25519 keypairs that authenticate each connection.

---

## Layer guide

| Layer | Guide |
|---|---|
| Computer & Tools | [Computer & Tools](layers.md) |
| Protocol | [Protocol Layer](protocol-layer.md) |
| Remote & MCP | [Remote & Integrations](remote-layer.md) |
| Data Flow | [Data Flow](data-flow.md) |
| Custom Tools | [Adding a Custom Tool](custom-tools.md) |
