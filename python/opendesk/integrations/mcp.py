"""MCP (Model Context Protocol) server adapter.

Exposes all opendesk tools as an MCP server that any MCP-compatible client
(Claude Desktop, Continue, Cursor, etc.) can connect to.

Usage — stdio transport (most common)::

    # Run as standalone server:
    python -m opendesk.integrations.mcp

    # Or via the installed script:
    opendesk-mcp

Usage — in-process::

    from opendesk.integrations.mcp import create_mcp_server
    from opendesk.registry import create_registry
    from mcp.server.stdio import stdio_server

    registry = create_registry()
    server = create_mcp_server(registry)

    async with stdio_server() as streams:
        await server.run(*streams, server.create_initialization_options())

Claude Desktop config (~/Library/Application Support/Claude/claude_desktop_config.json)::

    {
      "mcpServers": {
        "opendesk": {
          "command": "opendesk-mcp"
        }
      }
    }
"""

from __future__ import annotations

import asyncio
import base64
import json
import sys
from typing import Any

from opendesk.registry import ToolRegistry, create_registry
from opendesk.tools.base import ToolContext, allow_all_context


def create_mcp_server(
    registry: ToolRegistry | None = None,
    ctx: ToolContext | None = None,
) -> Any:
    """Create an MCP :class:`Server` wrapping all tools in *registry*.

    Parameters
    ----------
    registry:
        Tool registry to expose.  Defaults to :func:`~opendesk.registry.create_registry`.
    ctx:
        :class:`ToolContext` used for every tool call.
        Defaults to :func:`~opendesk.tools.base.allow_all_context`.

    Raises
    ------
    ImportError
        When the ``mcp`` package is not installed (``pip install 'opendesk[mcp]'``).
    """
    try:
        from mcp.server import Server
        from mcp import types as mcp_types
    except ImportError as exc:
        raise ImportError(
            "The 'mcp' package is required for MCP integration:\n"
            "  pip install 'opendesk[mcp]'\n"
            "  # or: pip install mcp"
        ) from exc

    if registry is None:
        registry = create_registry()
    if ctx is None:
        ctx = allow_all_context()

    server = Server("opendesk")

    @server.list_tools()
    async def list_tools() -> list[mcp_types.Tool]:
        return [
            mcp_types.Tool(
                name=tool.name,
                description=tool.description,
                inputSchema=tool.get_schema(),
            )
            for tool in registry.tools()
        ]

    @server.call_tool()
    async def call_tool(
        name: str,
        arguments: dict[str, Any],
    ) -> list[mcp_types.TextContent | mcp_types.ImageContent]:
        tool = registry.get(name)
        params = tool.parse_params(arguments)
        result = await tool.execute(ctx, params)

        contents: list[mcp_types.TextContent | mcp_types.ImageContent] = []

        # Text output
        if result.output:
            contents.append(
                mcp_types.TextContent(type="text", text=result.output)
            )

        # Binary attachments (screenshots, etc.)
        for att in result.attachments:
            if att.media_type.startswith("image/"):
                contents.append(
                    mcp_types.ImageContent(
                        type="image",
                        data=att.to_base64(),
                        mimeType=att.media_type,
                    )
                )
            else:
                # Non-image attachment — encode as data URL in text
                b64 = att.to_base64()
                contents.append(
                    mcp_types.TextContent(
                        type="text",
                        text=f"[Attachment: {att.filename} ({att.media_type})]\n"
                             f"data:{att.media_type};base64,{b64}",
                    )
                )

        if not contents:
            contents.append(
                mcp_types.TextContent(type="text", text="(no output)")
            )

        return contents

    return server


async def _run_stdio() -> None:
    """Run the MCP server over stdio."""
    try:
        from mcp.server.stdio import stdio_server
    except ImportError as exc:
        print(
            "ERROR: 'mcp' package not installed.\n"
            "Install with: pip install 'opendesk[mcp]'",
            file=sys.stderr,
        )
        raise SystemExit(1) from exc

    registry = create_registry()
    server = create_mcp_server(registry)

    async with stdio_server() as (read_stream, write_stream):
        await server.run(
            read_stream,
            write_stream,
            server.create_initialization_options(),
        )


def run_stdio_server() -> None:
    """Entry point for the ``opendesk-mcp`` CLI command."""
    asyncio.run(_run_stdio())


if __name__ == "__main__":
    run_stdio_server()
