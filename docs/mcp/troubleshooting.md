# Troubleshooting

**`opendesk-mcp` not found**

`cargo install --path opendesk-rs` puts the binaries in `~/.cargo/bin`. Make
sure that directory is on your PATH, or configure your client with the full
path to `opendesk-mcp`.

---

**`opendesk install` says `'claude' command not found`**

Install Claude Code first, or register the server manually with your client.

---

**Screenshot returns a permission error on macOS**

Go to System Settings → Privacy & Security → Screen Recording and add your terminal app (Terminal, iTerm2, etc.).
`opendesk check` reports the current state; `opendesk check --open` opens the settings pane.

---

**Mouse/keyboard actions do nothing on macOS**

Go to System Settings → Privacy & Security → Accessibility and add your terminal app.

---

**Input fails with `input refused: ...`**

An input guard is set (per call or via `opendesk_guard`) and the foreground
window does not match. Focus the right window, or clear the guard with
`opendesk_guard` `{ "action": "clear" }`. On non-Windows machines the
foreground window cannot be detected, so any guard refuses input.

---

**`opendesk_exec` reports `TIMED OUT`**

The command ran past `timeout_secs` (default 30). Its process tree was killed
and the output shown is partial. Raise `timeout_secs` for long commands.

---

Everything working? Browse the [Tools reference →](../tools/index.md) for full parameter docs.
