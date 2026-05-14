"""ScreenshotTool — capture the current screen via the active
:class:`~opendesk.computer.Computer`, with optional Set-of-Marks overlay."""

from __future__ import annotations

import asyncio
import io
import os
from typing import Any, List, Optional

from pydantic import Field

from opendesk.computer.types import Pixmap, Rect, UIElement
from opendesk.tools.base import Attachment, Tool, ToolContext, ToolResult


_INTERACTIVE_ROLES = {
    "AXButton", "AXTextField", "AXTextArea", "AXCheckBox", "AXRadioButton",
    "AXPopUpButton", "AXComboBox", "AXLink", "AXSearchField", "AXMenuButton",
    "AXDisclosureTriangle", "AXSlider", "AXMenuItem",
    "button", "checkbox", "radio button", "text", "entry", "combo box",
    "link", "slider", "menu item",
    "Button", "Edit", "CheckBox", "ComboBox", "ListItem",
    "MenuItem", "Hyperlink", "Slider", "Spinner", "TabItem",
}


def _flatten_interactive(root: UIElement, max_count: int = 150) -> list[dict[str, Any]]:
    """Walk a :class:`UIElement` tree and return the interactive leaves.

    Output matches :func:`opendesk.computer.marks.draw_som_marks`'s expected
    dict shape: ``{mark, role, label, x, y, w, h}``.
    """
    elements: list[dict[str, Any]] = []

    def visit(node: UIElement) -> None:
        if len(elements) >= max_count:
            return
        if node.bounds is not None and (node.role in _INTERACTIVE_ROLES or _looks_interactive(node)):
            b = node.bounds
            if b.width > 2 and b.height > 2:
                elements.append({
                    "mark": len(elements) + 1,
                    "role": node.role,
                    "label": (node.name or "")[:60],
                    "x": int(b.x), "y": int(b.y),
                    "w": int(b.width), "h": int(b.height),
                })
        for child in node.children:
            visit(child)

    visit(root)
    return elements


def _looks_interactive(node: UIElement) -> bool:
    role = node.role.lower()
    return any(k in role for k in ("button", "field", "check", "radio", "combo", "link", "menu", "slider"))


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
        from opendesk.computer.sandbox import ActionType, get_sandbox

        await ctx.check_permission(
            tool="screenshot", argument="capture screen",
            description="Take a screenshot of the current screen",
        )

        capture_rect = self._parse_region(params)
        if isinstance(capture_rect, ToolResult):
            return capture_rect

        sandbox = get_sandbox(ctx.session_id)
        if capture_rect and not sandbox.is_coordinate_allowed(
            int(capture_rect.x), int(capture_rect.y)
        ):
            return ToolResult(
                title="Screenshot denied",
                output="The requested region is outside the permitted screen area.",
                error=True,
            )

        try:
            pixmap: Pixmap = await ctx.computer.capture(region=capture_rect)
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

        png_bytes = pixmap.data
        width, height = pixmap.width, pixmap.height
        logical_w, logical_h = pixmap.logical_width, pixmap.logical_height
        scale_x, scale_y = pixmap.scale_x, pixmap.scale_y

        marks_summary: Optional[str] = None
        if params.marks or params.show_cursor:
            png_bytes, marks_summary, width, height = await self._overlay(
                ctx, png_bytes, scale_x, scale_y,
                draw_marks=params.marks, draw_cursor=params.show_cursor,
            )

        diff_summary: Optional[str] = None
        if sandbox.last_screenshot is not None and not params.zoom:
            try:
                from opendesk.computer.capture import diff_screenshots
                loop = asyncio.get_event_loop()
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

        saved_path: Optional[str] = None
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
        if params.show_cursor:
            try:
                pos = await ctx.computer.cursor_position()
                output_lines.append(f"Cursor position (logical): ({int(pos.x)}, {int(pos.y)})")
            except Exception:
                pass

        return ToolResult(
            title=f"Screenshot {width}x{height}{zoom_desc}{region_desc}",
            output="\n".join(output_lines),
            attachments=[Attachment("screenshot.png", png_bytes, "image/png")],
            metadata={"width": width, "height": height},
        )

    def _parse_region(self, params: "ScreenshotTool.Params"):
        if params.zoom:
            if len(params.zoom) != 4:
                return ToolResult(
                    title="Screenshot error",
                    output="zoom must have exactly 4 elements: [x0, y0, x1, y1]",
                    error=True,
                )
            x0, y0, x1, y1 = params.zoom
            return Rect(x=x0, y=y0, width=x1 - x0, height=y1 - y0)
        if params.region:
            if len(params.region) != 4:
                return ToolResult(
                    title="Screenshot error",
                    output="region must have exactly 4 elements: [x, y, width, height]",
                    error=True,
                )
            x, y, w, h = params.region
            return Rect(x=x, y=y, width=w, height=h)
        return None

    async def _overlay(
        self,
        ctx: ToolContext,
        png_bytes: bytes,
        scale_x: float,
        scale_y: float,
        *,
        draw_marks: bool,
        draw_cursor: bool,
    ) -> tuple[bytes, Optional[str], int, int]:
        """Render Set-of-Marks and / or cursor overlay onto ``png_bytes``."""
        try:
            from PIL import Image
        except ImportError:
            return png_bytes, None, 0, 0

        loop = asyncio.get_event_loop()
        pil_img = Image.open(io.BytesIO(png_bytes))
        marks_summary: Optional[str] = None

        if draw_marks:
            try:
                from opendesk.computer.marks import draw_som_marks
                tree = await ctx.computer.ui_tree()
                elements = _flatten_interactive(tree)
                pil_img, _mark_map, marks_summary = await loop.run_in_executor(
                    None, draw_som_marks, pil_img, elements, scale_x, scale_y,
                )
            except Exception:
                pass

        if draw_cursor:
            try:
                from opendesk.computer.marks import overlay_cursor
                pos = await ctx.computer.cursor_position()
                pil_img = overlay_cursor(pil_img, int(pos.x), int(pos.y), scale_x, scale_y)
            except Exception:
                pass

        buf = io.BytesIO()
        pil_img.save(buf, format="PNG", optimize=True)
        return buf.getvalue(), marks_summary, pil_img.width, pil_img.height
