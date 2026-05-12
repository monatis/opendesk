"""ScreenshotTool — capture the current screen."""

from __future__ import annotations

import asyncio
import os
from typing import List, Optional

from pydantic import Field

from opendesk.tools.base import Attachment, Tool, ToolContext, ToolResult


class ScreenshotTool(Tool):
    """Capture a screenshot of the full screen or a specific region."""

    name = "screenshot"
    description = (
        "Capture a screenshot of the current screen or a sub-region. "
        "Returns the image so you can observe the current UI state before "
        "deciding which action to take next. Call this frequently to verify "
        "that previous actions had the intended effect.\n\n"
        "Options:\n"
        "  show_cursor=true  — draw a red dot at the current cursor position\n"
        "  marks=true        — overlay numbered boxes on all interactive elements "
        "(Set-of-Marks); the output lists each mark so you can say "
        "'click mark 3' instead of guessing pixel coordinates\n"
        "  zoom=[x0,y0,x1,y1] — return a cropped close-up of a screen region\n"
        "  save_path         — write the PNG to disk"
    )

    class Params(Tool.Params):
        region: Optional[List[int]] = Field(
            default=None,
            description=(
                "Screen region to capture as [x, y, width, height] in pixels. "
                "Omit to capture the entire primary screen."
            ),
        )
        save_path: Optional[str] = Field(
            default=None,
            description="Absolute path where the PNG should be saved on disk.",
        )
        show_cursor: bool = Field(
            default=False,
            description="When true, overlays a red dot at the current cursor position.",
        )
        marks: bool = Field(
            default=False,
            description=(
                "When true, draws numbered bounding boxes (Set-of-Marks) over "
                "all interactive UI elements. Uses the platform accessibility API."
            ),
        )
        zoom: Optional[List[int]] = Field(
            default=None,
            description=(
                "Crop region as [x0, y0, x1, y1] in logical screen pixels. "
                "Returns a zoomed-in view. Use after a full screenshot to inspect "
                "small text or crowded UI areas."
            ),
        )

    async def execute(self, ctx: ToolContext, params: "ScreenshotTool.Params") -> ToolResult:
        from opendesk.computer.capture import capture_screen
        from opendesk.computer.sandbox import ActionType, get_sandbox

        await ctx.check_permission(
            tool="screenshot",
            argument="capture screen",
            description="Take a screenshot of the current screen",
        )

        capture_region = None
        if params.zoom:
            if len(params.zoom) != 4:
                return ToolResult(
                    title="Screenshot error",
                    output="zoom must have exactly 4 elements: [x0, y0, x1, y1]",
                    error=True,
                )
            x0, y0, x1, y1 = params.zoom
            capture_region = (x0, y0, x1 - x0, y1 - y0)
        elif params.region:
            if len(params.region) != 4:
                return ToolResult(
                    title="Screenshot error",
                    output="region must have exactly 4 elements: [x, y, width, height]",
                    error=True,
                )
            capture_region = tuple(params.region)

        sandbox = get_sandbox(ctx.session_id)
        if capture_region and not sandbox.is_coordinate_allowed(
            capture_region[0], capture_region[1]
        ):
            return ToolResult(
                title="Screenshot denied",
                output="The requested region is outside the permitted screen area.",
                error=True,
            )

        try:
            loop = asyncio.get_event_loop()
            png_bytes, width, height = await loop.run_in_executor(
                None, capture_screen, capture_region
            )
        except ImportError as exc:
            return ToolResult(title="Screenshot error", output=str(exc), error=True)
        except Exception as exc:
            await sandbox.record_action(
                ActionType.SCREENSHOT,
                params={"region": params.region, "zoom": params.zoom},
                error=str(exc),
            )
            return ToolResult(
                title="Screenshot error",
                output=f"Failed to capture screenshot: {exc}",
                error=True,
            )

        logical_w = None
        logical_h = None
        try:
            import pyautogui  # type: ignore[import-not-found]
            logical_w, logical_h = pyautogui.size()
        except Exception:
            pass

        scale_x = (width / logical_w) if logical_w else 1.0
        scale_y = (height / logical_h) if logical_h else 1.0

        marks_summary = None
        if params.marks or params.show_cursor:
            try:
                from PIL import Image
                import io as _io
                pil_img = Image.open(_io.BytesIO(png_bytes))

                if params.marks:
                    from opendesk.computer.marks import draw_som_marks, get_interactive_elements
                    elements = await loop.run_in_executor(None, get_interactive_elements, None)
                    pil_img, _mark_map, marks_summary = draw_som_marks(
                        pil_img, elements, scale_x, scale_y
                    )

                if params.show_cursor:
                    from opendesk.computer.marks import overlay_cursor
                    try:
                        import pyautogui  # type: ignore[import-not-found]
                        cx, cy = pyautogui.position()
                        pil_img = overlay_cursor(pil_img, cx, cy, scale_x, scale_y)
                    except Exception:
                        pass

                buf = _io.BytesIO()
                pil_img.save(buf, format="PNG", optimize=True)
                png_bytes = buf.getvalue()
                width, height = pil_img.size
            except ImportError:
                pass
            except Exception:
                pass

        diff_summary = None
        if sandbox.last_screenshot is not None and not params.zoom:
            try:
                from opendesk.computer.capture import diff_screenshots
                diff = await loop.run_in_executor(
                    None, diff_screenshots, sandbox.last_screenshot, png_bytes
                )
                diff_summary = diff["summary"]
            except Exception:
                pass

        sandbox.last_screenshot = png_bytes

        await sandbox.record_action(
            ActionType.SCREENSHOT,
            params={"region": params.region, "zoom": params.zoom,
                    "marks": params.marks, "show_cursor": params.show_cursor},
            result=f"{width}x{height}" + (f" | {diff_summary}" if diff_summary else ""),
        )

        saved_path = None
        if params.save_path:
            try:
                dest = os.path.expanduser(params.save_path)
                os.makedirs(
                    os.path.dirname(dest) if os.path.dirname(dest) else ".",
                    exist_ok=True,
                )
                with open(dest, "wb") as fh:
                    fh.write(png_bytes)
                saved_path = dest
            except Exception as exc:
                return ToolResult(
                    title="Screenshot save error",
                    output=(
                        f"Screenshot captured ({width}x{height}) but could not be "
                        f"saved to {params.save_path!r}: {exc}"
                    ),
                    attachments=[Attachment("screenshot.png", png_bytes, "image/png")],
                    metadata={"width": width, "height": height},
                    error=True,
                )

        zoom_desc = f" (zoom {params.zoom})" if params.zoom else ""
        region_desc = f" (region {params.region})" if params.region and not params.zoom else ""
        save_desc = f" -> saved to {saved_path}" if saved_path else ""
        logical_note = (
            f" (logical screen: {logical_w}x{logical_h})" if logical_w and logical_h else ""
        )

        output_lines = [
            f"Captured {width}x{height} screenshot{zoom_desc}{region_desc}{save_desc}.{logical_note}",
            f"Mouse coordinates: pass image_width={width}, image_height={height} "
            "to the mouse tool for correct Retina scaling.",
        ]
        if diff_summary:
            output_lines.append(f"Change detection vs previous screenshot: {diff_summary}")
        if marks_summary:
            output_lines.append(f"\nSet-of-Marks -- interactive elements:\n{marks_summary}")
        if params.show_cursor and logical_w:
            try:
                import pyautogui  # type: ignore[import-not-found]
                cx, cy = pyautogui.position()
                output_lines.append(f"Cursor position (logical): ({cx}, {cy})")
            except Exception:
                pass

        return ToolResult(
            title=f"Screenshot {width}x{height}{zoom_desc}{region_desc}",
            output="\n".join(output_lines),
            attachments=[Attachment("screenshot.png", png_bytes, "image/png")],
            metadata={"width": width, "height": height},
        )
