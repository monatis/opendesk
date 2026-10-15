# Install

Build and install from a checkout of the repository (requires a Rust
toolchain with edition 2024 support):

```bash
cargo install --path opendesk-rs
```

This installs two binaries into `~/.cargo/bin`:

- `opendesk` — the CLI (pairing, `serve`, `exec`, `install`, …)
- `opendesk-mcp` — the stdio MCP server

`opendesk mcp` runs the same MCP server as `opendesk-mcp`. Both accept
`--home DIR`, `--rendezvous URL` and `--rendezvous-token TOKEN`.

Verify:

```bash
opendesk-mcp --help
```

On macOS, grant Accessibility and Screen Recording permission (check with
`opendesk check`).

---

Next: register with your client — [Claude Code](claude-code.md) · [Claude Desktop](claude-desktop.md) · [Cursor](cursor.md) · [Continue](continue.md)
