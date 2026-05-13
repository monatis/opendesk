"""Execute a scheduled task — replay a procedure or run a natural language task."""

from __future__ import annotations

import asyncio
import logging
import os
from pathlib import Path

logger = logging.getLogger(__name__)


async def run_task(task: str, project_dir: Path) -> str:
    """Execute a task string and return a status message.

    If task starts with 'replay <name>', loads and executes the learned procedure
    directly using opendesk tools (no LLM needed).

    Otherwise, sends the task as a natural language instruction to Claude via
    the Anthropic SDK (requires ANTHROPIC_API_KEY env var).
    """
    task = task.strip()

    if task.lower().startswith("replay "):
        name = task[len("replay "):].strip()
        return await _run_replay(name, project_dir)
    else:
        return await _run_natural_language(task, project_dir)


async def _run_replay(name: str, project_dir: Path) -> str:
    """Load a learned procedure and execute it via Claude."""
    from opendesk.automation.storage import load_procedure

    proc = load_procedure(project_dir, name)
    if proc is None:
        return f"Error: no procedure found for '{name}'"

    steps = proc.get("steps", [])
    description = proc.get("description", "")
    procedure_text = proc.get("procedure", "")

    prompt = (
        f"Execute this learned task: {proc.get('task_name', name)}\n"
        f"Goal: {description}\n\n"
        f"Steps:\n" + "\n".join(f"  {i+1}. {s}" for i, s in enumerate(steps)) +
        f"\n\nProcedure:\n{procedure_text}\n\n"
        "Use the ui tool first, screenshot(marks=True) if needed, mouse as last resort. "
        "Adapt to the current environment."
    )

    return await _run_with_claude(prompt)


async def _run_natural_language(task: str, project_dir: Path) -> str:
    """Send a natural language task to Claude with opendesk tools."""
    return await _run_with_claude(task)


async def _run_with_claude(prompt: str) -> str:
    """Run a prompt through Claude with all opendesk tools available."""
    api_key = os.environ.get("ANTHROPIC_API_KEY")
    if not api_key:
        return "Error: ANTHROPIC_API_KEY environment variable not set"

    try:
        import anthropic
    except ImportError:
        return "Error: anthropic package not installed. Run: pip install anthropic"

    try:
        from opendesk.integrations.claude_code import ClaudeCodeAdapter
        from opendesk.registry import create_registry
        from opendesk.tools.base import allow_all_context

        registry = create_registry()
        adapter = ClaudeCodeAdapter(registry)
        client = anthropic.Anthropic(api_key=api_key)

        messages = [{"role": "user", "content": prompt}]

        result = await adapter.run_loop(
            client=client,
            model=os.environ.get("OPENDESK_MODEL", "claude-sonnet-4-6"),
            messages=messages,
            system=(
                "You are a desktop automation agent. Complete the task using the available tools. "
                "Use the ui tool first (click by element name). "
                "Use screenshot(marks=True) to see numbered elements if ui fails. "
                "Use mouse as last resort for pixel-level control."
            ),
        )
        return f"ok: {str(result)[:200]}"

    except Exception as e:
        return f"Error: {e}"
