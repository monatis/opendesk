# `mouse` — Mouse Control

Use pixel coordinates exactly as they appear in the most recent screenshot of
the same machine. They are mapped onto that machine's desktop automatically;
there are no `image_width` / `image_height` parameters.

## Actions

| Action | Parameters | Description |
|--------|-----------|-------------|
| `click` | `x, y` | Left-click |
| `double_click` | `x, y` | Double left-click |
| `triple_click` | `x, y` | Triple-click (select word/line) |
| `right_click` | `x, y` | Right-click |
| `middle_click` | `x, y` | Middle-click |
| `move` | `x, y` | Move without clicking |
| `scroll` | `x, y, direction, amount` | Scroll `amount` clicks (default 3) |
| `drag` | `x, y, end_x, end_y` | Click-and-drag |
| `left_down` | `x, y` | Press and hold the left button |
| `left_up` | `x, y` | Release the left button |
| `cursor_position` | — | Return the current cursor location |

Optional on every action: `settle_ms` (wait after the action), `guard`
(refuse unless the foreground window matches; see
[Input guard](index.md#input-guard)), and `peer`.

Only vertical scrolling is implemented: `direction: "down"` scrolls down and
any other value scrolls up.

## Ask Claude

> "Click the Save button"

> "Scroll down on the page"

> "Where is the cursor right now?"

## Example calls

```json
{ "action": "click", "x": 400, "y": 300 }
{ "action": "scroll", "x": 760, "y": 400, "direction": "down", "amount": 5 }
{ "action": "drag", "x": 100, "y": 100, "end_x": 500, "end_y": 300 }
{ "action": "click", "x": 400, "y": 300, "guard": { "title": "Notepad" } }
```

---

Next: [keyboard →](keyboard.md)
