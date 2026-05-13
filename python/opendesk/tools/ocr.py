"""OCRTool — extract visible text from any screen region.

Captures the region via :class:`~opendesk.computer.Computer` then runs the
OCR backend on the resulting pixmap locally, so OCR works even when the
captured machine is a remote :class:`RemoteComputer`.
"""

from __future__ import annotations

import asyncio
from typing import List, Optional

from pydantic import Field

from opendesk.computer.types import Rect
from opendesk.tools.base import Tool, ToolContext, ToolResult


class OCRTool(Tool):
    """Extract text from the screen or a screen region using OCR."""

    name = "ocr"
    description = (
        "Capture a screen region and extract all visible text using OCR. "
        "Useful for reading text in images, PDFs rendered to screen, or legacy apps. "
        "Backends: pytesseract (if installed), macOS Vision (macOS 11+), "
        "Windows WinRT (Windows 10+)."
    )

    class Params(Tool.Params):
        region: Optional[List[int]] = Field(
            default=None,
            description=(
                "Screen region as [x, y, width, height] in logical pixels. "
                "Omit to OCR the entire primary screen."
            ),
        )

    async def execute(self, ctx: ToolContext, params: "OCRTool.Params") -> ToolResult:
        from opendesk.computer.sandbox import ActionType, get_sandbox

        region_str = str(params.region) if params.region else "full screen"
        await ctx.check_permission(
            tool="ocr", argument=f"ocr {region_str}",
            description=f"Extract text via OCR from {region_str}",
        )

        sandbox = get_sandbox(ctx.session_id)

        rect: Optional[Rect] = None
        if params.region:
            if len(params.region) != 4:
                return ToolResult(
                    title="OCR error",
                    output="region must have exactly 4 elements: [x, y, width, height]",
                    error=True,
                )
            x, y, w, h = params.region
            rect = Rect(x=x, y=y, width=w, height=h)

        try:
            pixmap = await ctx.computer.capture(region=rect)
        except Exception as exc:
            await sandbox.record_action(ActionType.OCR, {"region": params.region}, error=str(exc))
            return ToolResult(title="OCR error", output=str(exc), error=True)

        try:
            from opendesk.computer.ocr import ocr_image
            loop = asyncio.get_event_loop()
            text = await loop.run_in_executor(
                None, ocr_image, pixmap.data, pixmap.width, pixmap.height
            )
        except Exception as exc:
            await sandbox.record_action(ActionType.OCR, {"region": params.region}, error=str(exc))
            return ToolResult(title="OCR error", output=str(exc), error=True)

        await sandbox.record_action(
            ActionType.OCR, {"region": params.region},
            result=f"{len(text)} chars extracted",
        )

        if text.startswith("OCR not available") or text.startswith("OCR error"):
            return ToolResult(title="OCR result", output=text, error=True)

        return ToolResult(
            title=f"OCR: {region_str}",
            output=text,
            metadata={"char_count": len(text)},
        )
