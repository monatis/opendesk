"""Claude Code / Anthropic SDK adapter.

Converts opencua tools into the format expected by the Anthropic API
(``anthropic.Anthropic().messages.create(tools=[...])``) and dispatches
tool-use blocks back to the correct tool.

Usage::

    import anthropic
    from opencua.integrations.claude_code import ClaudeCodeAdapter
    from opencua.registry import create_registry
    from opencua.tools.base import allow_all_context

    registry = create_registry()
    adapter = ClaudeCodeAdapter(registry, ctx=allow_all_context())

    client = anthropic.Anthropic()
    messages = [{"role": "user", "content": "Take a screenshot"}]

    while True:
        response = client.messages.create(
            model="claude-opus-4-6",
            max_tokens=4096,
            tools=adapter.tool_definitions(),
            messages=messages,
        )

        if response.stop_reason == "end_turn":
            print(response.content[0].text)
            break

        # Handle tool use
        tool_results = await adapter.handle_response(response)
        messages.append({"role": "assistant", "content": response.content})
        messages.append({"role": "user", "content": tool_results})

Also available as standalone helpers for one-shot usage:

    tool_defs = as_anthropic_tools(registry)
    result_block = await dispatch_tool_use(tool_use_block, registry, ctx)
"""

from __future__ import annotations

import asyncio
import base64
from typing import Any

from opencua.registry import ToolRegistry, create_registry
from opencua.tools.base import ToolContext, allow_all_context


def as_anthropic_tools(registry: ToolRegistry) -> list[dict[str, Any]]:
    """Return tool definitions in Anthropic's tool-use format.

    Compatible with ``anthropic.types.ToolParam``.
    """
    tools = []
    for tool in registry.tools():
        schema = tool.get_schema()
        # Anthropic expects input_schema, not parameters
        tools.append({
            "name": tool.name,
            "description": tool.description,
            "input_schema": schema,
        })
    return tools


async def dispatch_tool_use(
    tool_use_block: Any,
    registry: ToolRegistry,
    ctx: ToolContext,
) -> dict[str, Any]:
    """Execute a single tool-use block and return an Anthropic tool_result block.

    Parameters
    ----------
    tool_use_block:
        An ``anthropic.types.ToolUseBlock`` (or any object with ``.id``,
        ``.name``, ``.input`` attributes).
    registry:
        Tool registry to dispatch into.
    ctx:
        :class:`ToolContext` for the execution.

    Returns
    -------
    dict matching the ``tool_result`` content block format.
    """
    tool_id = tool_use_block.id
    tool_name = tool_use_block.name
    arguments = dict(tool_use_block.input)

    try:
        tool = registry.get(tool_name)
    except KeyError as exc:
        return {
            "type": "tool_result",
            "tool_use_id": tool_id,
            "is_error": True,
            "content": [{"type": "text", "text": str(exc)}],
        }

    try:
        params = tool.parse_params(arguments)
        result = await tool.execute(ctx, params)
    except Exception as exc:
        return {
            "type": "tool_result",
            "tool_use_id": tool_id,
            "is_error": True,
            "content": [{"type": "text", "text": f"Tool execution error: {exc}"}],
        }

    # Build content blocks
    content: list[dict[str, Any]] = []
    if result.output:
        content.append({"type": "text", "text": result.output})

    for att in result.attachments:
        if att.media_type.startswith("image/"):
            content.append({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": att.media_type,
                    "data": att.to_base64(),
                },
            })
        else:
            content.append({
                "type": "text",
                "text": f"[Attachment: {att.filename}]\ndata:{att.media_type};base64,{att.to_base64()}",
            })

    if not content:
        content = [{"type": "text", "text": "(no output)"}]

    return {
        "type": "tool_result",
        "tool_use_id": tool_id,
        "is_error": result.error,
        "content": content,
    }


class ClaudeCodeAdapter:
    """High-level adapter for the Anthropic Messages API agentic loop.

    Example::

        adapter = ClaudeCodeAdapter(create_registry())
        tools = adapter.tool_definitions()
        # pass to client.messages.create(tools=tools, ...)
        tool_results = await adapter.handle_response(response)
    """

    def __init__(
        self,
        registry: ToolRegistry | None = None,
        ctx: ToolContext | None = None,
    ) -> None:
        self.registry = registry or create_registry()
        self.ctx = ctx or allow_all_context()

    def tool_definitions(self) -> list[dict[str, Any]]:
        """Return tool defs to pass to ``messages.create(tools=...)``."""
        return as_anthropic_tools(self.registry)

    async def handle_response(self, response: Any) -> list[dict[str, Any]]:
        """Process all tool_use blocks in *response* and return tool_result blocks.

        Returns a list of tool_result content blocks ready to be sent as the
        next ``user`` message.
        """
        tool_use_blocks = [
            block for block in response.content
            if hasattr(block, "type") and block.type == "tool_use"
        ]

        if not tool_use_blocks:
            return []

        results = await asyncio.gather(*[
            dispatch_tool_use(block, self.registry, self.ctx)
            for block in tool_use_blocks
        ])
        return list(results)

    async def run_loop(
        self,
        client: Any,
        model: str,
        messages: list[dict[str, Any]],
        system: str | None = None,
        max_tokens: int = 8192,
        max_iterations: int = 20,
    ) -> str:
        """Run the full agentic loop until the model stops using tools.

        Parameters
        ----------
        client:
            An ``anthropic.Anthropic()`` or ``anthropic.AsyncAnthropic()`` client.
        model:
            Model ID, e.g. ``"claude-opus-4-6"``.
        messages:
            Initial messages list (mutated in place).
        system:
            Optional system prompt.
        max_tokens:
            Max tokens per response.
        max_iterations:
            Safety limit on tool-use iterations.

        Returns
        -------
        The final text response from the model.
        """
        kwargs: dict[str, Any] = {
            "model": model,
            "max_tokens": max_tokens,
            "tools": self.tool_definitions(),
            "messages": messages,
        }
        if system:
            kwargs["system"] = system

        for _ in range(max_iterations):
            # Support both sync and async clients
            if asyncio.iscoroutinefunction(client.messages.create):
                response = await client.messages.create(**kwargs)
            else:
                loop = asyncio.get_event_loop()
                response = await loop.run_in_executor(
                    None, lambda: client.messages.create(**kwargs)
                )

            messages.append({"role": "assistant", "content": response.content})

            if response.stop_reason != "tool_use":
                # Extract final text
                for block in response.content:
                    if hasattr(block, "type") and block.type == "text":
                        return block.text
                return ""

            tool_results = await self.handle_response(response)
            messages.append({"role": "user", "content": tool_results})
            kwargs["messages"] = messages

        return "(max_iterations reached)"
