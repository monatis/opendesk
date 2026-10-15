# `screenshot` — Screen Capture

## Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `region` | `[x, y, w, h]` | full screen | Capture only this screen region |
| `zoom` | `[x0, y0, x1, y1]` | null | Crop a region for close-up inspection (takes precedence over `region`) |
| `show_cursor` | bool | false | Also report the current cursor position in the text output |
| `format` | `"png"` \| `"jpeg"` | `png` locally, `jpeg` for remote peers | Image format |
| `quality` | int | 75 | JPEG quality, 1–100 |
| `save_path` | string | null | Also write the image to this path |
| `peer` | string | default peer | Machine to capture |

The result is the image plus a line such as `Captured 1920x1080 screenshot.`

Coordinates you read off the most recent screenshot (including a `zoom` or
`region` capture) can be passed straight to the [`mouse`](mouse.md) tool; they
are mapped onto the desktop automatically, including monitors left of or above
the primary one.

The Rust version has no Set-of-Marks overlay (`marks`) and no pixel-diff
change report.

## Ask Claude

> "Take a screenshot and tell me what's on my screen"

> "Show me what's in the top-right corner of the screen"

> "Save a screenshot to C:\temp\before.png"

## Example calls

```json
{}
{ "zoom": [100, 200, 400, 350] }
{ "format": "jpeg", "quality": 60, "peer": "work-pc" }
{ "save_path": "/tmp/before.png" }
```

---

Next: [mouse →](mouse.md)
