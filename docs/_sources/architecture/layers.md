# Computer & Tools Layers

## `computer/`

### `LocalComputer` (`local.rs`)

The capability surface of the current machine, built mostly on the `xa11y`
crate (screen capture, accessibility trees, input simulation) and `arboard`
(clipboard). Main operations:

* **Observe:** `screenshot_format` (PNG/JPEG, optional region),
  `cursor_position`, `ui_tree`, `ui_get_value`, `app_list`, `clipboard_read`.
* **Act:** `mouse_move` / `mouse_click` / `mouse_down` / `mouse_up` /
  `mouse_scroll`, `keyboard_type` / `keyboard_press` / `keyboard_hotkey` /
  `keyboard_hold` / `keyboard_sequence`, `ui_click` / `ui_type`,
  `app_open` / `app_close` / `app_focus`, `clipboard_write`,
  `execute_command` / `execute_argv`.
* **Coordinates:** `image_to_screen` maps a point in the last screenshot onto
  the desktop (including monitors at negative coordinates), which is why the
  `mouse` tool takes screenshot pixels directly.

On Windows, keyboard input is sent with `SendInput`: text as Unicode packets
(no clipboard), single characters and chords as real key events resolved on
the foreground window's keyboard layout.

### Helpers

* `keys.rs` — chord parsing (`split_chord`: `+` separator, so `ctrl+-` and
  `ctrl++` work) and layout lookup (`layout_key`, Windows).
* `foreground.rs` — the foreground window (Windows) and `WindowGuard`, the
  check behind the `guard` argument and `opendesk_guard`.
* `process.rs` — command execution with a timeout that kills the whole
  process tree (Job Object on Windows, process group on Unix) and keeps the
  output printed so far.
* `privacy.rs` — physical input lock and blackout curtain for
  `opendesk_privacy` (remote peers only).
* `permissions.rs` — macOS Accessibility / Screen Recording checks
  (`opendesk check`).

### `RemoteComputer` (`remote/client.rs`)

Mirrors the `LocalComputer` operations as async methods that send a protocol
request (`display.capture`, `input.pointer`, `input.key`, …) to the peer and
decode the result. It carries the session's input guard and attaches it to
`input.*` calls so the peer checks its own foreground window.

### Server-side dispatch (`remote/server.rs`)

`dispatch_call(computer, method, params)` maps each method name to the
matching `LocalComputer` call. See
[Session & Methods](../protocol/session.md) for the method list.

---

## Tools (`mcp/server.rs`)

There is no separate tool layer or tool trait: each MCP tool is a `match`
arm in `McpServer::call_tool`, with a local branch (calls `LocalComputer`)
and a remote branch (calls `RemoteComputer`). `list_tools` returns the JSON
schemas. Tools that are always local: `audit` (reads the server-side audit
log), `learn` (`~/.opendesk/procedures/`), `schedule`
(`automation/scheduler.rs`, `<cwd>/.opendesk/schedules.json`).

Not present in the Rust version: Set-of-Marks screenshots, pixel-diff
feedback, OCR, input recording, and action replay.

---

Next: [Protocol Layer →](protocol-layer.md)
