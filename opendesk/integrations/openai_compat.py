"""OpenAI-compatible function calling adapter.

Converts opendesk tools into OpenAI function definitions and dispatches
``tool_calls`` from the response back to the correct tool.

Works with the OpenAI SDK and any API that follows the OpenAI tool-calling
format (Together AI, Groq, Ollama, LiteLLM, vLLM, etc.).

Usage::

    from openai import OpenAI
    from opendesk.integrations.openai_compat import OpenAIAdapter
    from opendesk.registry import create_registry

    client = OpenAI()
    adapter = OpenAIAdapter(create_registry())

    messages = [{"role": "user", "content": "Take a screenshot"}]

    while True:
        response = client.chat.completions.create(
            model="gpt-4o",
            tools=adapter.tool_definitions(),
            messages=messages,
        )
        choice = response.choices[0]
        messages.append(choice.message)

        if choice.finish_reason != "tool_calls":
            print(choice.message.content)
            break

        tool_messages = await adapter.handle_response(choice.message)
        messages.extend(tool_messages)
"""

from __future__ import annotations

import asyncio
import json
from typing import Any

from opendesk.registry import ToolRegistry, create_registry
from opendesk.tools.base import ToolContext, allow_all_context


def as_openai_tools(registry: ToolRegistry) -> list[dict[str, Any]]:
    """Return tool definitions in OpenAI's function-calling format."""
    tools = []
    for tool in registry.tools():
        schema = tool.get_schema()
        # OpenAI wraps schema in a "function" object
        tools.append({
            "type": "function",
            "function": {
                "name": tool.name,
                "description": tool.description,
                "parameters": schema,
            },
        })
    return tools


async def dispatch_tool_call(
    tool_call: Any,
    registry: ToolRegistry,
    ctx: ToolContext,
) -> dict[str, Any]:
    """Execute a single OpenAI tool_call and return a tool message dict.

    Parameters
    ----------
    tool_call:
        An OpenAI ``ChatCompletionMessageToolCall`` (or compatible object with
        ``.id``, ``.function.name``, ``.function.arguments``).
    registry, ctx:
        Registry and context for execution.

    Returns
    -------
    A dict with ``role="tool"`` ready to append to messages.
    """
    call_id = tool_call.id
    tool_name = tool_call.function.name

    try:
        arguments = json.loads(tool_call.function.arguments)
    except json.JSONDecodeError as exc:
        return {
            "role": "tool",
            "tool_call_id": call_id,
            "content": f"Invalid JSON arguments: {exc}",
        }

    try:
        tool = registry.get(tool_name)
        params = tool.parse_params(arguments)
        result = await tool.execute(ctx, params)
    except Exception as exc:
        return {
            "role": "tool",
            "tool_call_id": call_id,
            "content": f"Tool execution error: {exc}",
        }

    # OpenAI tool messages only support text content.
    # For image attachments, encode as markdown data-URL (best effort).
    parts: list[str] = []
    if result.output:
        parts.append(result.output)

    for att in result.attachments:
        if att.media_type.startswith("image/"):
            parts.append(
                f"![screenshot](data:{att.media_type};base64,{att.to_base64()})"
            )
        else:
            parts.append(
                f"[Attachment: {att.filename} — {att.media_type}]"
            )

    return {
        "role": "tool",
        "tool_call_id": call_id,
        "content": "\n".join(parts) if parts else "(no output)",
    }


class OpenAIAdapter:
    """High-level adapter for the OpenAI Chat Completions agentic loop.

    Example::

        adapter = OpenAIAdapter(create_registry())
        final_text = await adapter.run_loop(client, "gpt-4o", messages)
    """

    def __init__(
        self,
        registry: ToolRegistry | None = None,
        ctx: ToolContext | None = None,
    ) -> None:
        self.registry = registry or create_registry()
        self.ctx = ctx or allow_all_context()

    def tool_definitions(self) -> list[dict[str, Any]]:
        """Return tool defs to pass to ``chat.completions.create(tools=...)``."""
        return as_openai_tools(self.registry)

    async def handle_response(self, message: Any) -> list[dict[str, Any]]:
        """Execute all tool_calls in *message* and return tool result messages."""
        if not hasattr(message, "tool_calls") or not message.tool_calls:
            return []
        results = await asyncio.gather(*[
            dispatch_tool_call(tc, self.registry, self.ctx)
            for tc in message.tool_calls
        ])
        return list(results)

    async def run_loop(
        self,
        client: Any,
        model: str,
        messages: list[dict[str, Any]],
        system: str | None = None,
        max_tokens: int = 4096,
        max_iterations: int = 20,
        **kwargs: Any,
    ) -> str:
        """Run the full agentic loop until the model stops calling tools."""
        if system:
            messages = [{"role": "system", "content": system}, *messages]

        create_kwargs: dict[str, Any] = {
            "model": model,
            "max_tokens": max_tokens,
            "tools": self.tool_definitions(),
            "messages": messages,
            **kwargs,
        }

        for _ in range(max_iterations):
            if asyncio.iscoroutinefunction(client.chat.completions.create):
                response = await client.chat.completions.create(**create_kwargs)
            else:
                loop = asyncio.get_event_loop()
                response = await loop.run_in_executor(
                    None, lambda: client.chat.completions.create(**create_kwargs)
                )

            choice = response.choices[0]
            messages.append(choice.message)

            if choice.finish_reason != "tool_calls":
                return choice.message.content or ""

            tool_messages = await self.handle_response(choice.message)
            messages.extend(tool_messages)
            create_kwargs["messages"] = messages

        return "(max_iterations reached)"
