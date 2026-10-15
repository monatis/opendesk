# Tools Reference

These are the tools the `opendesk-mcp` server (and `opendesk mcp`) exposes over
MCP. They are defined in `opendesk-rs/src/mcp/server.rs` (`list_tools`). There
is no in-process tool API to import: agents call the tools over MCP, and Rust
programs can use the `opendesk-rs` crate directly (see
[Programmatic Use](../remote/programmatic.md)).

**Tool priority rule:** `ui` first; `screenshot` + `mouse` for surfaces with no
accessible elements.

| Tool | Description |
|---|---|
| [`ui`](ui.md) | Accessibility-based UI interaction: list elements, click by name, type, read values |
| [`screenshot`](screenshot.md) | Screen capture (PNG or JPEG), optional region / zoom |
| [`mouse`](mouse.md) | Pixel-level mouse control using screenshot coordinates |
| [`keyboard`](keyboard.md) | Type text, press keys and chords, hold keys, timed sequences |
| [`app`](app.md) | Open, close, focus, and list applications |
| [`clipboard`](clipboard.md) | Read and write clipboard text |
| `opendesk_exec` | Run a shell command on the target machine (see below) |
| [`audit`](audit.md) | Show this machine's server-side audit log |
| [`learn`](learn.md) | Save and replay written procedures |
| `schedule` | Store named schedules (see [Scheduler](../automation/scheduler.md)) |
| [`ocr`](ocr.md) | Not available in the Rust version |

Admin tools (`opendesk_peers`, `opendesk_use`, `opendesk_status`,
`opendesk_discover`, `opendesk_describe`, `opendesk_capabilities`,
`opendesk_disconnect`, `opendesk_guard`, `opendesk_privacy`, `opendesk_view`)
are described in [Remote: MCP Integration](../remote/mcp.md).

---

## Routing: the `peer` argument

`screenshot`, `mouse`, `keyboard`, `app`, `ui`, `clipboard` and `opendesk_exec`
accept an optional `peer` string. Without it, the call goes to the session's
default peer (`opendesk_use`, then the persistent default from
`opendesk peers default`, then `local`). `audit`, `learn` and `schedule` always
run on the machine hosting the MCP server.

## Input guard

`mouse` and `keyboard` accept an optional `guard` object:

```json
{ "action": "type", "text": "hello", "guard": { "process": "notepad" } }
```

`title` is a case-insensitive substring of the foreground window title;
`process` is the executable name (with or without `.exe`). The check runs on
the machine receiving the input, right before it is sent. If the foreground
window does not match, the call fails with `input refused: ...` and nothing is
typed or clicked. `opendesk_guard` (`action`: `set` / `clear` / `show`, plus
`title` / `process`) sets a session default that applies to every `mouse` and
`keyboard` call, local or remote; a per-call `guard` overrides it.

Foreground-window detection is implemented for Windows only. On other
platforms the foreground window cannot be determined, so any non-empty guard
refuses input.

## `opendesk_exec`

| Parameter | Type | Default | Description |
|---|---|---|---|
| `command` | string | required | Command string to execute |
| `shell` | string | PowerShell on Windows, `sh` elsewhere | `powershell`, `cmd`, `bash`, or `sh` |
| `timeout_secs` | number | 30 | Rounded up to whole seconds |
| `peer` | string | default peer | Where to run it |

The result reports the exit code, stdout and stderr. On timeout the whole
process tree is killed (a Job Object on Windows, a process group on Unix), and
the output collected so far is returned with the status
`TIMED OUT (process tree killed; output below is partial)`.

The CLI equivalent is `opendesk exec COMMAND [--peer NAME] [--shell SH]
[--timeout S]`, or `opendesk exec --file script.ps1 ...` to send the contents
of a local script file instead (this sidesteps quoting in the local shell).
The CLI exits with the command's exit code, or `124` on timeout.

---

Start with the primary interaction tool: [ui →](ui.md)
