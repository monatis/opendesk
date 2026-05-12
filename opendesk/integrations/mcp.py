"""MCP (Model Context Protocol) server adapter.

Exposes opendesk to any MCP-compatible client (Claude Desktop, Claude Code,
Continue, Cursor) as a single server that can drive either the local machine
or any paired remote peer.

Two flavours of tool
--------------------
1. **Computer-use tools** — screenshot, mouse, keyboard, ui, app, clipboard,
   ocr.  Each accepts an optional ``peer`` argument:

   * Omitted → the MCP session's default peer (or local if none set).
   * ``"local"`` → always the local machine.
   * Any trusted peer name → that remote machine.

2. **Session admin tools** — ``opendesk_peers``, ``opendesk_discover``,
   ``opendesk_use``, ``opendesk_status``, ``opendesk_capabilities``,
   ``opendesk_disconnect``.  Agents use these to choose which machine to
   drive and to inspect what's available.

Pairing is intentionally not exposed: the 6-digit code should be typed by a
human reading it off the controlled machine, not flow through the agent's
text channel.  Use ``opendesk pair`` / ``opendesk pair-with`` from the CLI.

Usage — stdio transport (most common)::

    # Run as standalone server:
    python -m opendesk.integrations.mcp

    # Or via the installed script:
    opendesk-mcp

Claude Desktop config::

    {
      "mcpServers": {
        "opendesk": { "command": "opendesk-mcp" }
      }
    }
"""

from __future__ import annotations

import asyncio
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Optional

from opendesk.integrations.mcp_session import LOCAL, MCPSession, MCPSessionError
from opendesk.registry import ToolRegistry, create_registry
from opendesk.tools.base import ToolContext


# Tools that operate on a Computer's state — these get peer routing.  The
# rest (learn, schedule, audit) act on local session state and aren't
# remotable.
PEER_AWARE_TOOLS: set[str] = {
    "screenshot", "mouse", "keyboard", "app", "ui", "clipboard", "ocr",
}


# ---------------------------------------------------------------------------
# Server factory
# ---------------------------------------------------------------------------


@dataclass
class ToolEntry:
    """Library-neutral description of a tool the MCP server exposes."""

    name: str
    description: str
    schema: dict[str, Any]


@dataclass
class TextResult:
    text: str


@dataclass
class ImageResult:
    data_base64: str
    mime_type: str


ToolResult = list  # list[TextResult | ImageResult]


class MCPDispatcher:
    """The MCP integration's brain — listing tools and dispatching calls.

    Library-neutral: returns plain dataclasses that :func:`create_mcp_server`
    converts to ``mcp.types.*``.  Tests exercise this class directly without
    importing the ``mcp`` package.
    """

    def __init__(
        self,
        registry: ToolRegistry,
        session: MCPSession,
        permission_handler: Optional[Callable] = None,
    ) -> None:
        self._registry = registry
        self._session = session
        self._permission_handler = permission_handler

    @property
    def session(self) -> MCPSession:
        return self._session

    async def list_tools(self) -> list[ToolEntry]:
        entries: list[ToolEntry] = []
        for tool in self._registry.tools():
            schema = tool.get_schema()
            description = tool.description
            if tool.name in PEER_AWARE_TOOLS:
                schema = _augment_peer_field(schema)
                description = _augment_peer_description(description)
            entries.append(ToolEntry(tool.name, description, schema))
        for admin in _admin_tool_definitions():
            entries.append(ToolEntry(admin.name, admin.description, admin.schema))
        return entries

    async def call_tool(self, name: str, arguments: dict[str, Any]) -> ToolResult:
        admin = _ADMIN_HANDLERS.get(name)
        if admin is not None:
            try:
                text = await admin(self._session, dict(arguments))
            except (MCPSessionError, Exception) as exc:
                text = f"ERROR: {exc}"
            return [TextResult(text)]

        try:
            tool = self._registry.get(name)
        except KeyError:
            return [TextResult(f"ERROR: unknown tool {name!r}")]

        peer_arg: Optional[str] = None
        arguments = dict(arguments)
        if tool.name in PEER_AWARE_TOOLS and "peer" in arguments:
            raw = arguments.pop("peer")
            peer_arg = raw if isinstance(raw, str) and raw.strip() else None

        try:
            computer, computer_name = await self._session.resolve(peer_arg)
        except MCPSessionError as exc:
            return [TextResult(f"ERROR: {exc}")]

        ctx = ToolContext(
            session_id=f"mcp-{computer_name}",
            permission_handler=self._permission_handler,
            computer=computer,
        )
        try:
            params = tool.parse_params(arguments)
        except Exception as exc:
            return [TextResult(f"ERROR: invalid arguments: {exc}")]

        result = await tool.execute(ctx, params)

        out: ToolResult = []
        prefix = "" if computer_name == LOCAL else f"[on {computer_name}] "
        if result.output:
            out.append(TextResult(prefix + result.output))
        for att in result.attachments:
            if att.media_type.startswith("image/"):
                out.append(ImageResult(att.to_base64(), att.media_type))
            else:
                b64 = att.to_base64()
                out.append(TextResult(
                    f"[Attachment: {att.filename} ({att.media_type})]\n"
                    f"data:{att.media_type};base64,{b64}"
                ))
        if not out:
            out.append(TextResult("(no output)"))
        return out


def create_mcp_server(
    registry: ToolRegistry | None = None,
    session: MCPSession | None = None,
    *,
    home: Path | None = None,
    permission_handler: Optional[Callable] = None,
) -> Any:
    """Create an MCP :class:`Server` exposing all opendesk tools.

    Raises :class:`ImportError` if the ``mcp`` package isn't installed.
    """
    try:
        from mcp.server import Server
        from mcp import types as mcp_types
    except ImportError as exc:
        raise ImportError(
            "The 'mcp' package is required: pip install 'opendesk[mcp]'"
        ) from exc

    if registry is None:
        registry = create_registry()
    if session is None:
        session = MCPSession(home=home)

    dispatcher = MCPDispatcher(registry, session, permission_handler)
    server = Server("opendesk")

    @server.list_tools()
    async def list_tools() -> list[mcp_types.Tool]:
        return [
            mcp_types.Tool(name=e.name, description=e.description, inputSchema=e.schema)
            for e in await dispatcher.list_tools()
        ]

    @server.call_tool()
    async def call_tool(
        name: str, arguments: dict[str, Any],
    ) -> list[Any]:
        out = await dispatcher.call_tool(name, arguments)
        converted: list[Any] = []
        for item in out:
            if isinstance(item, TextResult):
                converted.append(mcp_types.TextContent(type="text", text=item.text))
            elif isinstance(item, ImageResult):
                converted.append(mcp_types.ImageContent(
                    type="image", data=item.data_base64, mimeType=item.mime_type,
                ))
        return converted

    # Allow callers to access state.
    server._opendesk_dispatcher = dispatcher  # type: ignore[attr-defined]
    return server


# ---------------------------------------------------------------------------
# Schema / description augmentation
# ---------------------------------------------------------------------------


def _augment_peer_field(schema: dict[str, Any]) -> dict[str, Any]:
    """Return a copy of *schema* with an optional ``peer`` field added."""
    schema = dict(schema)
    properties = dict(schema.get("properties") or {})
    properties["peer"] = {
        "type": "string",
        "description": (
            "Optional. Name of the peer to run this action on. "
            "Use 'local' for the local machine, or any name from `opendesk_peers`. "
            "When omitted, falls back to the session's default peer "
            "(see `opendesk_status` / `opendesk_use`)."
        ),
    }
    schema["properties"] = properties
    # Don't add to required.
    return schema


def _augment_peer_description(description: str) -> str:
    return (
        description
        + "\n\nRoutes to the session's default peer unless 'peer' is provided. "
          "See `opendesk_peers` to list available targets."
    )


# ---------------------------------------------------------------------------
# Admin tools
# ---------------------------------------------------------------------------


@dataclass
class _AdminTool:
    name: str
    description: str
    schema: dict[str, Any]


def _admin_tool_definitions() -> list[_AdminTool]:
    return [
        _AdminTool(
            name="opendesk_peers",
            description=(
                "List the peers available to this MCP session. Returns the local "
                "machine and every paired remote peer. The current default is "
                "marked with [current]; open connections are marked [active]."
            ),
            schema={"type": "object", "properties": {}, "additionalProperties": False},
        ),
        _AdminTool(
            name="opendesk_discover",
            description=(
                "Discover opendesk peers advertising themselves on the LAN via "
                "mDNS. May take a few seconds. Only paired peers can be used; "
                "discovered-but-unpaired peers will need pairing via the CLI "
                "(`opendesk pair-with <host> <code>`)."
            ),
            schema={
                "type": "object",
                "properties": {
                    "timeout": {
                        "type": "number",
                        "description": "Seconds to wait for mDNS responses. Defaults to 2.",
                    },
                },
                "additionalProperties": False,
            },
        ),
        _AdminTool(
            name="opendesk_use",
            description=(
                "Set the default peer for subsequent Computer-use tool calls. "
                "Pass 'local' (or omit `peer`) to revert to controlling this machine."
            ),
            schema={
                "type": "object",
                "properties": {
                    "peer": {
                        "type": "string",
                        "description": "Peer name (from `opendesk_peers`), or 'local'.",
                    },
                },
                "required": ["peer"],
                "additionalProperties": False,
            },
        ),
        _AdminTool(
            name="opendesk_status",
            description=(
                "Show the current default peer and the list of peers with open "
                "cached connections in this session."
            ),
            schema={"type": "object", "properties": {}, "additionalProperties": False},
        ),
        _AdminTool(
            name="opendesk_capabilities",
            description=(
                "Show what the given peer's backend can do (input devices, "
                "screen capture, filesystem, etc.). Omit `peer` for the current "
                "default."
            ),
            schema={
                "type": "object",
                "properties": {
                    "peer": {
                        "type": "string",
                        "description": "Peer name or 'local'.",
                    },
                },
                "additionalProperties": False,
            },
        ),
        _AdminTool(
            name="opendesk_disconnect",
            description=(
                "Close a cached remote connection. Pass `peer` to close one, "
                "or omit it to close all remote connections."
            ),
            schema={
                "type": "object",
                "properties": {
                    "peer": {"type": "string"},
                },
                "additionalProperties": False,
            },
        ),
    ]


async def _admin_peers(session: MCPSession, args: dict[str, Any]) -> str:
    from opendesk.protocol.auth import TrustedPeers
    trusted = TrustedPeers(session.home).list()
    effective_name, source = session.effective_peer()
    active = set(session.active_peer_names())

    lines: list[str] = []
    if source == "ambiguous":
        lines.append(
            "No default peer set — multiple peers paired.  "
            "Pick one with `opendesk_use <name>`, or pass `peer:` on each call.",
        )
        lines.append("")
    lines.append("Available peers:")
    local_tags: list[str] = []
    if source == "local":
        local_tags.append("default")
    lines.append(f"  local{_tags(local_tags)}")
    for p in trusted:
        tags = []
        if effective_name == p.name:
            tags.append(f"default ({source})")
        if p.name in active:
            tags.append("active")
        lines.append(f"  {p.name}{_tags(tags)}  ({p.fingerprint})")
    if not trusted:
        lines.append("  (no trusted remote peers — pair via `opendesk pair-with` on the CLI)")
    return "\n".join(lines)


async def _admin_discover(session: MCPSession, args: dict[str, Any]) -> str:
    try:
        from opendesk.remote.discovery import discover
    except ImportError as exc:
        return f"Discovery unavailable: {exc}"
    timeout = float(args.get("timeout") or 2.0)
    peers = await discover(timeout=timeout)
    if not peers:
        return f"No opendesk peers found on the LAN (within {timeout:.1f}s)."

    from opendesk.protocol.auth import TrustedPeers
    trusted = {p.public_key: p.name for p in TrustedPeers(session.home).list()}

    lines = [f"Found {len(peers)} peer(s) on the LAN:"]
    for p in peers:
        paired = trusted.get(p.public_key)
        tag = f"  [paired as {paired}]" if paired else "  [NOT paired]"
        lines.append(f"  {p.name}  {p.host}:{p.port}  {p.fingerprint}{tag}")
    return "\n".join(lines)


async def _admin_use(session: MCPSession, args: dict[str, Any]) -> str:
    name = args.get("peer")
    if not isinstance(name, str) or not name.strip():
        raise MCPSessionError("`peer` is required (use 'local' to revert)")
    session.use_peer(name if name != LOCAL else None)
    target = session.current_peer or LOCAL
    return f"Default peer is now: {target}"


async def _admin_status(session: MCPSession, args: dict[str, Any]) -> str:
    effective_name, source = session.effective_peer()
    if source == "ambiguous":
        parts = [
            "Default peer: (none — multiple peers paired)",
            "  Pick one with `opendesk_use <name>` or pass `peer:` on each call.",
        ]
    else:
        target = effective_name or LOCAL
        parts = [f"Default peer: {target} ({source})"]
        if source == "implicit":
            parts.append(
                "  (single paired peer — pairing another will require an explicit default)",
            )
    active = session.active_peer_names()
    parts.append(f"Open connections: {', '.join(active) if active else 'none'}")
    return "\n".join(parts)


async def _admin_capabilities(session: MCPSession, args: dict[str, Any]) -> str:
    name = args.get("peer")
    computer, resolved = await session.resolve(name)
    manifest = computer.capabilities()
    caps = sorted(c.value for c in manifest.capabilities)
    lines = [
        f"Peer: {resolved}",
        f"Backend: {manifest.backend}",
        f"Protocol: {manifest.protocol_version}",
        "Capabilities:",
    ]
    lines.extend(f"  - {c}" for c in caps)
    if manifest.limits:
        lines.append("Limits:")
        lines.append("  " + json.dumps(manifest.limits))
    return "\n".join(lines)


async def _admin_disconnect(session: MCPSession, args: dict[str, Any]) -> str:
    name = args.get("peer")
    n = await session.disconnect(name)
    if name:
        return f"Closed connection to {name}." if n else f"No active connection to {name}."
    return f"Closed {n} connection(s)."


_ADMIN_HANDLERS = {
    "opendesk_peers": _admin_peers,
    "opendesk_discover": _admin_discover,
    "opendesk_use": _admin_use,
    "opendesk_status": _admin_status,
    "opendesk_capabilities": _admin_capabilities,
    "opendesk_disconnect": _admin_disconnect,
}


def _tags(tags: list[str]) -> str:
    return "  [" + ", ".join(tags) + "]" if tags else ""


# ---------------------------------------------------------------------------
# stdio entry point
# ---------------------------------------------------------------------------


async def _run_stdio() -> None:
    try:
        from mcp.server.stdio import stdio_server
    except ImportError as exc:
        print(
            "ERROR: 'mcp' package not installed.  pip install 'opendesk[mcp]'",
            file=sys.stderr,
        )
        raise SystemExit(1) from exc

    registry = create_registry()
    session = MCPSession()
    server = create_mcp_server(registry, session)

    try:
        async with stdio_server() as (read_stream, write_stream):
            await server.run(
                read_stream, write_stream, server.create_initialization_options(),
            )
    finally:
        await session.aclose()


def run_stdio_server() -> None:
    """Entry point for the ``opendesk-mcp`` console script."""
    asyncio.run(_run_stdio())


if __name__ == "__main__":
    run_stdio_server()
