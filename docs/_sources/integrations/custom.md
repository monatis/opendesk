# Custom / Generic Integration

The Python tool registry, `ToolContext`, and `configure_sandbox` of the
earlier Python version are **not available** in the Rust version. There are
two supported routes:

## 1. MCP over stdio (any language)

Start `opendesk-mcp` as a child process and speak MCP JSON-RPC on its
stdin/stdout:

- `tools/list` returns every tool with its JSON `inputSchema`
  (see the [Tools reference](../tools/index.md)).
- `tools/call` with `{"name": ..., "arguments": {...}}` runs one; screenshots
  come back as MCP image content.

`opendesk-mcp` accepts `--home DIR`, `--rendezvous URL` and
`--rendezvous-token TOKEN`.

## 2. The Rust crate

Link `opendesk-rs` and call `LocalComputer` or `RemoteComputer` directly. See
[Programmatic Use](../remote/programmatic.md).

## Restricting what the agent can do

There is no allowed-apps / screen-region sandbox in the Rust version. The
controls that exist are:

- the [input guard](../tools/index.md#input-guard) (`guard` argument and
  `opendesk_guard`), which refuses mouse / keyboard input unless the expected
  window is in the foreground;
- on a controlled machine, pairing (only trusted peers can connect), the
  single-controller rule, `opendesk unpair`, and the
  [audit log](../tools/audit.md).

---

From here you might want the [Tools reference →](../tools/index.md), or
[Architecture →](../architecture/index.md) to understand how the pieces fit together.
