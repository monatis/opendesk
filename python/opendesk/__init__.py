"""opendesk — Open Computer Use Agent framework.

Quick start::

    from opendesk import create_registry, allow_all_context

    registry = create_registry()
    ctx = allow_all_context()

    # Take a screenshot
    screenshot_tool = registry.get("screenshot")
    result = await screenshot_tool.execute(ctx, screenshot_tool.Params(marks=True))

    # Click a button by name (no coordinates needed)
    ui_tool = registry.get("ui")
    await ui_tool.execute(ctx, ui_tool.Params(action="click", app="Safari", title="Go"))

Integrations::

    # MCP server (Claude Desktop, Continue, Cursor, ...)
    from opendesk.integrations.mcp import create_mcp_server

    # Anthropic / Claude Code
    from opendesk.integrations.claude_code import ClaudeCodeAdapter

    # OpenAI function calling
    from opendesk.integrations.openai_compat import OpenAIAdapter

    # LangChain
    from opendesk.integrations.langchain_compat import as_langchain_tools
"""

from opendesk.computer import (
    Capability,
    CapabilityManifest,
    CapabilityUnsupported,
    Computer,
    LocalComputer,
)
from opendesk.registry import ToolRegistry, create_registry, create_minimal_registry
from opendesk.tools.base import (
    Tool,
    ToolContext,
    ToolResult,
    Attachment,
    PermissionDeniedError,
    allow_all_context,
    interactive_context,
)

__version__ = "0.1.0"
__all__ = [
    # Registry
    "create_registry",
    "create_minimal_registry",
    "ToolRegistry",
    # Tool framework
    "Tool",
    "ToolContext",
    "ToolResult",
    "Attachment",
    "PermissionDeniedError",
    "allow_all_context",
    "interactive_context",
    # Computer surface
    "Computer",
    "LocalComputer",
    "Capability",
    "CapabilityManifest",
    "CapabilityUnsupported",
]
