# `ui` — Accessibility-based UI Interaction

The primary interaction tool. Finds elements through the platform
accessibility API (via the `xa11y` crate) and acts on them, so no pixel
coordinates are needed.

## Actions

| Action | Main params | Description |
|--------|-------------|-------------|
| `get_tree` | `app`, `max_depth` | List the accessible elements of the app window (default depth 8) |
| `click` | `app`, `title` / `role` / `selector` | Click an element by its label, role, or selector |
| `click_menu` | `app`, `menu`, `menu_item` | Click a menu item, e.g. File → Save |
| `type` | `app`, `text`, optional `title` / `selector` | Type into a named element, or into the focused one |
| `press_key` | `app`, `key`, `modifiers` | Focus the app, then press a key or chord |
| `get_value` | `app`, `title` / `selector` | Read the current value of an element |

Other parameters: `role` (narrows the search; role names are platform-specific,
e.g. `Button` / `Edit` on Windows), `selector` (an accessibility selector such
as `button[name="Save"]`), `window_index`, and `peer`.

## Ask Claude

> "Click the Save button in Notepad"

> "Open the File menu and click New Window"

> "What buttons are visible in this app?"

> "What's the current value of the address bar?"

## Example calls

```json
{ "action": "get_tree", "app": "Notepad" }
{ "action": "click", "app": "Notepad", "title": "Save" }
{ "action": "click_menu", "app": "Notepad", "menu": "File", "menu_item": "New" }
{ "action": "press_key", "app": "Notepad", "key": "s", "modifiers": ["ctrl"] }
```

When an app uses custom rendering (canvas apps, games, some Electron apps),
`get_tree` may return little or nothing; fall back to `screenshot` and the
`mouse` tool.

---

Next: [screenshot →](screenshot.md)
