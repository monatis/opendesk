"""Learn tool — record and replay computer tasks via global input capture."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Literal, Optional

from pydantic import BaseModel

from opendesk.tools.base import Tool, ToolContext, ToolResult

# Module-level recorder singleton — only one recording at a time
_active_recorder: Any = None


class LearnTool(Tool):
    name = "learn"
    description = (
        "Record and replay computer tasks. "
        "Actions:\n"
        "- start: begin recording mouse/keyboard/screenshots for a named task\n"
        "- stop: stop recording; returns trajectory summary and screenshots for you to summarize into a procedure, then call learn(save) with the JSON\n"
        "- save: save a procedure JSON returned from stop\n"
        "- replay: load a saved procedure and return step-by-step replay instructions\n"
        "- list: list all saved procedures\n\n"
        "Typical workflow: learn(start) → user performs task → learn(stop) → "
        "summarize the trajectory → learn(save) → later: learn(replay)"
    )

    class Params(BaseModel):
        action: Literal["start", "stop", "save", "replay", "list"]
        task_name: Optional[str] = None
        procedure: Optional[str] = None  # JSON string for save action

    async def execute(self, ctx: ToolContext, params: "LearnTool.Params") -> ToolResult:
        await ctx.check_permission(
            self.name,
            params.action,
            f"learn action={params.action}",
        )

        if params.action == "start":
            return await self._start(params.task_name)
        if params.action == "stop":
            return await self._stop()
        if params.action == "save":
            return await self._save(params.task_name, params.procedure)
        if params.action == "replay":
            return await self._replay(params.task_name)
        if params.action == "list":
            return await self._list()

        return ToolResult(title="learn", output=f"Unknown action: {params.action}")

    # ------------------------------------------------------------------

    async def _start(self, task_name: Optional[str]) -> ToolResult:
        global _active_recorder
        if not task_name:
            return ToolResult(title="learn", output="Error: task_name is required for start")
        if _active_recorder is not None:
            return ToolResult(
                title="learn",
                output="A recording is already active. Call learn(stop) first."
            )
        try:
            from opendesk.learn.recorder import LearnRecorder
        except ImportError:
            return ToolResult(title="learn", output="Error: pynput is required. Run: pip install pynput")

        _active_recorder = LearnRecorder(task_name)
        _active_recorder.start()
        return ToolResult(
            title="learn",
            output=f"Recording started for task '{task_name}'. "
                   f"Perform the task now, then call learn(action=stop) when done."
        )

    async def _stop(self) -> ToolResult:
        global _active_recorder
        if _active_recorder is None:
            return ToolResult(title="learn", output="No active recording. Start one with learn(action=start, task_name=...)")

        from opendesk.learn.trajectory import build_display_summary, build_summarization_content

        trajectory = _active_recorder.stop()
        _active_recorder = None

        summary = build_display_summary(trajectory)

        # Build MCP-friendly content: text summary + key screenshots
        from opendesk.tools.base import Attachment

        attachments = []
        if trajectory.initial_screenshot:
            attachments.append(Attachment(
                content=_b64_to_bytes(trajectory.initial_screenshot),
                media_type="image/jpeg",
                filename="initial_screen.jpg",
            ))

        snap_count = 0
        for i, ev in enumerate(trajectory.events):
            if ev.screenshot_after and snap_count < 6:
                if ev.action_type in ("click", "right_click") or (
                    ev.action_type == "key" and ev.key in ("return", "escape")
                ):
                    attachments.append(Attachment(
                        content=_b64_to_bytes(ev.screenshot_after),
                        media_type="image/jpeg",
                        filename=f"step_{i+1}.jpg",
                    ))
                    snap_count += 1

        output = (
            f"{summary}\n\n"
            "---\n"
            "Now summarize this recording into a procedure JSON and call:\n"
            "  learn(action=save, task_name=..., procedure='{\"task_name\":\"...\",\"description\":\"...\","
            "\"steps\":[...],\"procedure\":\"...\"}')\n"
            "Guidelines: describe goals, not file paths or app names. "
            "Write steps an agent can replay in any environment."
        )

        return ToolResult(title="learn", output=output, attachments=attachments)

    async def _save(self, task_name: Optional[str], procedure: Optional[str]) -> ToolResult:
        if not task_name:
            return ToolResult(title="learn", output="Error: task_name is required for save")
        if not procedure:
            return ToolResult(title="learn", output="Error: procedure JSON string is required for save")

        try:
            data = json.loads(procedure)
        except json.JSONDecodeError as e:
            return ToolResult(title="learn", output=f"Error: invalid JSON — {e}")

        from opendesk.learn.storage import save_procedure

        path = save_procedure(Path.cwd(), task_name, data)
        return ToolResult(title="learn", output=f"Procedure '{task_name}' saved to {path}")

    async def _replay(self, task_name: Optional[str]) -> ToolResult:
        if not task_name:
            return ToolResult(title="learn", output="Error: task_name is required for replay")

        from opendesk.learn.storage import load_procedure

        proc = load_procedure(Path.cwd(), task_name)
        if proc is None:
            procs_dir = Path.cwd() / ".opendesk" / "learned"
            return ToolResult(
                title="learn",
                output=f"No procedure found for '{task_name}'. "
                       f"Check {procs_dir} or run learn(list) to see available tasks."
            )

        steps = proc.get("steps", [])
        procedure_text = proc.get("procedure", "")
        description = proc.get("description", "")

        steps_text = "\n".join(f"  {i+1}. {s}" for i, s in enumerate(steps))
        prompt = (
            f"Replaying task: {proc.get('task_name', task_name)}\n"
            f"Goal: {description}\n\n"
            f"Steps:\n{steps_text}\n\n"
            f"Procedure:\n{procedure_text}\n\n"
            "Execute each step using the available tools (ui, screenshot, mouse, keyboard, app). "
            "Use the ui tool first, screenshot(marks=True) if needed, mouse as last resort. "
            "Adapt to the current environment — do not rely on specific file paths or app versions."
        )
        return ToolResult(title="learn", output=prompt)

    async def _list(self) -> ToolResult:
        from opendesk.learn.storage import list_procedures

        procs = list_procedures(Path.cwd())
        if not procs:
            return ToolResult(
                title="learn",
                output="No learned procedures yet. Record one with learn(action=start, task_name=...)"
            )

        lines = ["Learned procedures:", ""]
        for p in procs:
            desc = f" — {p['description']}" if p.get("description") else ""
            lines.append(f"  {p['name']}{desc}")
        return ToolResult(title="learn", output="\n".join(lines))


def _b64_to_bytes(b64: str) -> bytes:
    import base64
    return base64.b64decode(b64)
