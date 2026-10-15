# `app` — Application Control

## Actions

| Action | Parameters | Description |
|--------|-----------|-------------|
| `open` | `name` | Launch an application by name or full executable path |
| `close` | `name` | Close the application |
| `focus` | `name` | Bring its window to the foreground |
| `list` | — | List running applications / windows |

All actions accept an optional `peer`.

## Ask Claude

> "Open Notepad"

> "Close the browser"

> "Switch to File Explorer"

> "What apps are currently running?"

## Example calls

```json
{ "action": "open", "name": "notepad" }
{ "action": "focus", "name": "Notepad" }
{ "action": "list" }
{ "action": "close", "name": "Notepad", "peer": "work-pc" }
```

---

Next: [clipboard →](clipboard.md)
