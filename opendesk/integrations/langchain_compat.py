"""LangChain tool adapter.

Wraps opendesk tools as ``langchain_core.tools.BaseTool`` instances so they
can be used in LangChain agents, chains, and LCEL pipelines.

Usage::

    from langchain_core.tools import tool
    from langchain_openai import ChatOpenAI
    from langgraph.prebuilt import create_react_agent

    from opendesk.integrations.langchain_compat import as_langchain_tools
    from opendesk.registry import create_registry

    lc_tools = as_langchain_tools(create_registry())

    llm = ChatOpenAI(model="gpt-4o")
    agent = create_react_agent(llm, lc_tools)

    result = agent.invoke({"messages": [("user", "Take a screenshot")]})

Requires: ``pip install langchain-core``
"""

from __future__ import annotations

import asyncio
from typing import Any, Type

from opendesk.registry import ToolRegistry, create_registry
from opendesk.tools.base import ToolContext, allow_all_context


def as_langchain_tools(
    registry: ToolRegistry | None = None,
    ctx: ToolContext | None = None,
) -> list[Any]:
    """Return all tools in *registry* as :class:`langchain_core.tools.BaseTool` instances.

    Raises
    ------
    ImportError
        When ``langchain-core`` is not installed.
    """
    try:
        from langchain_core.tools import BaseTool
        from pydantic import BaseModel, Field as PydanticField
    except ImportError as exc:
        raise ImportError(
            "langchain-core is required for LangChain integration:\n"
            "  pip install 'opendesk[langchain]'\n"
            "  # or: pip install langchain-core"
        ) from exc

    if registry is None:
        registry = create_registry()
    if ctx is None:
        ctx = allow_all_context()

    lc_tools = []
    for tool in registry.tools():
        lc_tools.append(_wrap_tool(tool, ctx))
    return lc_tools


def _wrap_tool(tool: Any, ctx: ToolContext) -> Any:
    """Wrap a single opendesk :class:`~opendesk.tools.base.Tool` as a LangChain tool."""
    try:
        from langchain_core.tools import BaseTool
    except ImportError as exc:
        raise ImportError("langchain-core required: pip install langchain-core") from exc

    tool_name = tool.name
    tool_description = tool.description
    tool_schema = tool.get_schema()

    class _WrappedTool(BaseTool):
        name: str = tool_name
        description: str = tool_description

        def _run(self, **kwargs: Any) -> str:
            result = asyncio.run(tool.execute(ctx, tool.parse_params(kwargs)))
            return _format_result(result)

        async def _arun(self, **kwargs: Any) -> str:
            result = await tool.execute(ctx, tool.parse_params(kwargs))
            return _format_result(result)

        @property
        def args(self) -> dict[str, Any]:
            return tool_schema.get("properties", {})

    return _WrappedTool()


def _format_result(result: Any) -> str:
    """Convert a ToolResult to a string for LangChain."""
    parts = []
    if result.output:
        parts.append(result.output)
    for att in result.attachments:
        if att.media_type.startswith("image/"):
            parts.append(f"[Screenshot captured: {att.filename} ({len(att.content)} bytes)]")
        else:
            parts.append(f"[Attachment: {att.filename}]")
    return "\n".join(parts) if parts else "(no output)"
