# `keyboard` — Keyboard Input

## Actions

| Action | Required | Description |
|--------|----------|-------------|
| `type` | `text` | Type a string (full Unicode; does not use the clipboard) |
| `press` | `key` | Press a key or a chord: `enter`, `f5`, `a`, `ctrl+shift+s` |
| `hotkey` | `keys` (or `key`) | Send a chord: `["ctrl","c"]`, or `"ctrl+c"` |
| `hold` | `key`, `hold_duration` | Hold a key for N seconds (default 1.0), then release |
| `sequence` | `steps` | Run several steps atomically on the target |

Optional on every action: `settle_ms`, `guard` (refuse unless the foreground
window matches; see [Input guard](index.md#input-guard)), and `peer`.

## Keys and chords

- Chords use `+` as the separator: `ctrl+shift+s`, `alt+f4`. A `+` or `-` in
  key position is the key itself, so `ctrl+-`, `ctrl++`, `-` and `+` all work.
  `-` is accepted as a separator only when the chord has no `+` (`ctrl-s`).
- A single character (`press` with `key: "a"`) is pressed as the real key that
  produces it on the target's keyboard layout, not injected as a Unicode
  packet. For example, on a Turkish-Q layout `i` is not the `I` key. If the
  layout cannot type the character, it falls back to typing it. Layout lookup
  is implemented on Windows.
- `hotkey` accepts an array of key names, and each entry may itself be a chord
  (`["ctrl", "shift+s"]`).
- Named keys include `enter`, `escape`, `tab`, `backspace`, `delete`, `up`,
  `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `f1`–`f12`,
  `ctrl`, `alt`, `shift`, `win` / `cmd`, `capslock`, `space`.

## Sequences

Each step is an object with one of `hotkey` (array or chord string), `key`, or
`text`, plus an optional `delay_ms` to wait after the step:

```json
{
  "action": "sequence",
  "steps": [
    { "hotkey": ["ctrl", "l"] },
    { "text": "example.com", "delay_ms": 50 },
    { "key": "enter" }
  ]
}
```

## Ask Claude

> "Type 'Hello World' into the active window"

> "Press Ctrl+S to save"

> "Zoom out with Ctrl+-"

## Example calls

```json
{ "action": "type", "text": "Hello, 世界!" }
{ "action": "press", "key": "enter" }
{ "action": "press", "key": "ctrl+shift+s", "guard": { "process": "code" } }
{ "action": "hotkey", "keys": ["ctrl", "c"] }
{ "action": "hold", "key": "shift", "hold_duration": 0.5 }
```

---

Next: [app →](app.md)
