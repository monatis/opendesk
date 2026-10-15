# `clipboard` — Clipboard Read/Write

## Actions

| Action | Parameters | Description |
|--------|-----------|-------------|
| `read` | — | Return the current clipboard text |
| `write` | `text` | Set the clipboard text |

Both accept an optional `peer`.

## Ask Claude

> "What's in my clipboard?"

> "Copy this text to my clipboard: 'hello world'"

## Example calls

```json
{ "action": "read" }
{ "action": "write", "text": "Hello from opendesk" }
```

To paste, follow a `write` with the [`keyboard`](keyboard.md) tool
(`{"action": "press", "key": "ctrl+v"}`).

---

Next: [audit →](audit.md)
