# Advanced: Server Options & Restrictions

## Server options

`opendesk-mcp` (and `opendesk mcp`) accept:

| Flag | Purpose |
|---|---|
| `--home DIR` | Identity / trusted-peers directory (default `~/.opendesk`) |
| `--rendezvous URL` | Rendezvous relay for reaching peers across the Internet |
| `--rendezvous-token TOKEN` | Token for that relay |

Logging goes to stderr and is off by default; set `RUST_LOG` (for example
`RUST_LOG=debug`) to enable it.

The working directory of the MCP server process is where the `schedule` tool
stores `.opendesk/schedules.json`.

---

## Embedding and permission policies

The Python version's in-process server (`create_mcp_server`), custom
permission handlers, and `configure_sandbox` (allowed apps / screen region)
are **not available** in the Rust version. To embed opendesk in a Rust
program, use the crate directly — see
[Programmatic Use](../remote/programmatic.md).

---

## Restricting where input lands

Use the input guard so mouse and keyboard calls are refused unless the
expected window is in the foreground:

```json
// opendesk_guard
{ "action": "set", "process": "notepad" }
```

Every later `mouse` / `keyboard` call in the session (local or remote) is then
checked on the target machine right before the input is sent. A per-call
`guard` argument overrides the default; `{ "action": "clear" }` removes it.
Foreground detection is Windows-only; see
[Input guard](../tools/index.md#input-guard).

---

## Audit log

The `audit` tool and `opendesk audit` show the audit log that `opendesk serve`
writes on a controlled machine (`~/.opendesk/audit/YYYY-MM-DD.jsonl`). Calls
the MCP server performs on the local machine are not logged. See
[`audit`](../tools/audit.md).

---

Something not working? Check the [Troubleshooting guide →](troubleshooting.md)
