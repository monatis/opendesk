"""Schedule tool — add, remove, list, and run scheduled tasks."""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Literal, Optional

from pydantic import BaseModel

from opendesk.tools.base import Tool, ToolContext, ToolResult


class ScheduleTool(Tool):
    name = "schedule"
    description = (
        "Schedule tasks to run automatically on a timer.\n\n"
        "Actions:\n"
        "- add: schedule a task. task can be natural language ('take a screenshot and save it') "
        "or a learned procedure ('replay expense-report'). "
        "timing examples: 'every 30m', 'every 2h', 'every day at 09:00', 'every friday at 17:00'\n"
        "- remove: remove a schedule by name\n"
        "- list: show all scheduled tasks with their next run time\n"
        "- run: run a scheduled task immediately (for testing)\n\n"
        "After adding schedules, start the background runner with: opendesk scheduler start"
    )

    class Params(BaseModel):
        action: Literal["add", "remove", "list", "run"]
        name: Optional[str] = None      # schedule name
        task: Optional[str] = None      # task to run
        timing: Optional[str] = None    # when to run

    async def execute(self, ctx: ToolContext, params: "ScheduleTool.Params") -> ToolResult:
        await ctx.check_permission(self.name, params.action, f"schedule action={params.action}")

        if params.action == "add":
            return self._add(params.name, params.task, params.timing)
        if params.action == "remove":
            return self._remove(params.name)
        if params.action == "list":
            return self._list()
        if params.action == "run":
            return await self._run(params.name)

        return ToolResult(title="schedule", output=f"Unknown action: {params.action}")

    def _add(self, name: Optional[str], task: Optional[str], timing: Optional[str]) -> ToolResult:
        if not name:
            return ToolResult(title="schedule", output="Error: name is required")
        if not task:
            return ToolResult(title="schedule", output="Error: task is required")
        if not timing:
            return ToolResult(title="schedule", output="Error: timing is required")

        from opendesk.schedule.store import ScheduleStore

        try:
            store = ScheduleStore(Path.cwd())
            entry = store.add(name=name, task=task, timing=timing)
        except ValueError as e:
            return ToolResult(title="schedule", output=f"Error: {e}")

        trigger = f"cron: {entry.cron}" if entry.cron else f"every {entry.interval_seconds}s"
        return ToolResult(
            title="schedule",
            output=(
                f"Scheduled '{name}' ({trigger})\n"
                f"Task: {task}\n\n"
                f"Start the background runner to activate it:\n"
                f"  opendesk scheduler start"
            ),
        )

    def _remove(self, name: Optional[str]) -> ToolResult:
        if not name:
            return ToolResult(title="schedule", output="Error: name is required")

        from opendesk.schedule.store import ScheduleStore

        store = ScheduleStore(Path.cwd())
        if store.remove(name):
            return ToolResult(title="schedule", output=f"Removed schedule '{name}'")
        return ToolResult(title="schedule", output=f"No schedule found with name '{name}'")

    def _list(self) -> ToolResult:
        from opendesk.schedule.store import ScheduleStore
        import time

        store = ScheduleStore(Path.cwd())
        entries = store.all()

        if not entries:
            return ToolResult(
                title="schedule",
                output=(
                    "No schedules yet.\n"
                    "Add one with: schedule(action=add, name=..., task=..., timing=...)"
                ),
            )

        lines = ["Scheduled tasks:", ""]
        for e in entries:
            status = "enabled" if e.enabled else "disabled"
            last = "never" if not e.last_run else _ago(e.last_run)
            last_result = f" [{e.last_status}]" if e.last_status else ""
            lines.append(f"  {e.name}  ({e.timing})  {status}")
            lines.append(f"    task: {e.task}")
            lines.append(f"    last run: {last}{last_result}")
            lines.append("")

        lines.append("Run 'opendesk scheduler start' to activate scheduled tasks.")
        return ToolResult(title="schedule", output="\n".join(lines))

    async def _run(self, name: Optional[str]) -> ToolResult:
        if not name:
            return ToolResult(title="schedule", output="Error: name is required")

        from opendesk.schedule.store import ScheduleStore
        from opendesk.schedule.runner import run_task

        store = ScheduleStore(Path.cwd())
        entry = store.get(name)
        if not entry:
            return ToolResult(title="schedule", output=f"No schedule found with name '{name}'")

        status = await run_task(entry.task, Path.cwd())
        store.update_run(name, status)
        return ToolResult(title="schedule", output=f"Ran '{name}': {status}")


def _ago(ts: float) -> str:
    import time
    diff = int(time.time() - ts)
    if diff < 60:
        return f"{diff}s ago"
    if diff < 3600:
        return f"{diff // 60}m ago"
    if diff < 86400:
        return f"{diff // 3600}h ago"
    return f"{diff // 86400}d ago"
