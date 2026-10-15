# Adding a Custom Tool

The Rust version has no plugin API or tool registry; the Python
`Tool` / `create_registry` interface is not available. Adding a tool means
changing `opendesk-rs/src/mcp/server.rs`:

1. Add its JSON schema to `McpServer::list_tools`.
2. Handle its name in `McpServer::call_tool` — a local branch using
   `LocalComputer`, and, if it should work on peers, a remote branch using
   `RemoteComputer`.
3. If the remote branch needs a capability the protocol doesn't have yet, add
   a method to `RemoteComputer` (`remote/client.rs`) and a matching arm to
   `dispatch_call` (`remote/server.rs`) on the controlled side.

Often no new tool is needed: `opendesk_exec` already runs arbitrary commands
locally or on a peer.

Alternatively, run your own MCP server next to `opendesk-mcp` in the same
client, or call the crate from your own Rust program (see
[Programmatic Use](../remote/programmatic.md)).

---

That covers the architecture. For the low-level wire format, see the [Protocol reference →](../protocol/index.md)
