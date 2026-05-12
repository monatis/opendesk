"""Audit tool — expose the session audit log inside any MCP or agent session."""

from __future__ import annotations

import json
from typing import Literal

from pydantic import Field

from opendesk.tools.base import Tool, ToolContext, ToolResult


class AuditTool(Tool):
    """Return the audit log for the current session.

    Useful in MCP / Claude Code sessions where the sandbox cannot be
    accessed programmatically.  Ask the agent "show audit log" or
    "show audit summary" to call this tool.
    """

    name = "audit"
    description = (
        "Show the session audit log — every action taken so far in this session. "
        "Use format='summary' for a one-line count, or format='full' for the "
        "complete timestamped log."
    )

    class Params(Tool.Params):
        format: Literal["summary", "full"] = Field(
            default="full",
            description="'summary' returns a one-line action count; "
                        "'full' returns the complete timestamped log.",
        )
        session_id: str | None = Field(
            default=None,
            description="Session to inspect. Defaults to the current session.",
        )

    async def execute(self, ctx: ToolContext, params: "AuditTool.Params") -> ToolResult:
        from opendesk.computer.sandbox import get_sandbox

        session_id = params.session_id or ctx.session_id
        sandbox = get_sandbox(session_id)

        if params.format == "summary":
            text = sandbox.summary()
            return ToolResult(title="Audit summary", output=text)

        log = sandbox.export_audit_log()
        if not log:
            return ToolResult(
                title="Audit log",
                output=f"No actions recorded yet for session '{session_id}'.",
            )

        lines: list[str] = [f"Audit log — session '{session_id}' ({len(log)} actions)\n"]
        for entry in log:
            import datetime
            ts = datetime.datetime.fromtimestamp(entry["timestamp"]).strftime(
                "%Y-%m-%d %H:%M:%S"
            )
            error_tag = f"  ERROR: {entry['error']}" if entry["error"] else ""
            params_str = json.dumps(entry["params"], ensure_ascii=False)
            lines.append(
                f"[{ts}] {entry['action']:<20} {params_str}{error_tag}"
            )

        return ToolResult(title="Audit log", output="\n".join(lines))
