"""Base classes for opendesk tools.

All tools derive from :class:`Tool` and produce :class:`ToolResult` objects.
The system is deliberately framework-agnostic: no dependency on any LLM SDK,
agent harness, or specific runtime.

Integrations (MCP, Claude Code, OpenAI, LangChain) are in
:mod:`opendesk.integrations` and consume these base classes.

Example::

    from opendesk.tools.base import Tool, ToolContext, ToolResult

    class MyTool(Tool):
        name = "my_tool"
        description = "Does something useful."

        class Params(Tool.Params):
            message: str

        async def execute(self, ctx: ToolContext, params: "MyTool.Params") -> ToolResult:
            return ToolResult(title="Done", output=f"Got: {params.message}")
"""

from __future__ import annotations

import asyncio
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Awaitable, Callable

if TYPE_CHECKING:
    from opendesk.computer.base import Computer


# ---------------------------------------------------------------------------
# Result types
# ---------------------------------------------------------------------------


@dataclass
class Attachment:
    """A binary file attached to a :class:`ToolResult` (e.g. a screenshot PNG)."""

    filename: str
    content: bytes
    media_type: str = "application/octet-stream"

    def to_base64(self) -> str:
        import base64
        return base64.b64encode(self.content).decode("ascii")


@dataclass
class ToolResult:
    """The output of a tool execution.

    Attributes
    ----------
    title:
        Short human-readable label (shown in UIs).
    output:
        Plain-text content returned to the LLM or caller.
    error:
        True when the tool encountered an error.  The LLM should retry or
        choose a different action rather than continuing blindly.
    attachments:
        Binary files (images, PDFs, …) to be forwarded to the LLM if the
        integration supports multi-modal content.
    metadata:
        Arbitrary key/value pairs for programmatic consumers (not shown to LLM).
    """

    title: str
    output: str
    error: bool = False
    attachments: list[Attachment] = field(default_factory=list)
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        """Serialise to a plain dict (no binary blobs — use to_dict_b64 for those)."""
        return {
            "title": self.title,
            "output": self.output,
            "error": self.error,
            "metadata": self.metadata,
        }

    def to_dict_b64(self) -> dict[str, Any]:
        """Serialise including base-64 encoded attachments."""
        d = self.to_dict()
        d["attachments"] = [
            {
                "filename": att.filename,
                "media_type": att.media_type,
                "content_b64": att.to_base64(),
            }
            for att in self.attachments
        ]
        return d


# ---------------------------------------------------------------------------
# Context
# ---------------------------------------------------------------------------

PermissionHandler = Callable[[str, str, str], Awaitable[None]]
"""``async (tool_name, argument, description) -> None``

Raise :class:`PermissionDeniedError` to block an action, or return normally
to allow it.  Called before every tool execution.
"""


class PermissionDeniedError(RuntimeError):
    """Raised by a :data:`PermissionHandler` to block a tool action."""


def _default_local_computer() -> "Computer":
    """Build a :class:`LocalComputer` lazily to avoid import cycles."""
    from opendesk.computer.local import LocalComputer
    return LocalComputer()


@dataclass
class ToolContext:
    """Runtime context injected into every tool execution.

    Attributes
    ----------
    session_id:
        Opaque string that identifies the current conversation / session.
        Used by the sandbox for audit logging.
    permission_handler:
        Optional async callable ``(tool, argument, description) -> None``.
        Return normally to allow; raise :class:`PermissionDeniedError` to block.
        When ``None``, all actions are allowed automatically.
    metadata:
        Arbitrary caller-supplied data (e.g. user identity, org context).
    computer:
        The :class:`~opendesk.computer.Computer` instance every tool talks to.
        Defaults to a :class:`~opendesk.computer.LocalComputer`.  Swap in a
        ``RemoteComputer`` to make every tool target a different machine
        without changing any tool code.
    """

    session_id: str = "default"
    permission_handler: PermissionHandler | None = None
    metadata: dict[str, Any] = field(default_factory=dict)
    computer: "Computer" = field(default_factory=_default_local_computer)

    async def check_permission(
        self,
        tool: str,
        argument: str,
        description: str,
    ) -> None:
        """Ask the permission handler whether the action is allowed.

        If no handler is set, the action is allowed automatically (headless /
        fully-autonomous mode).
        """
        if self.permission_handler is not None:
            await self.permission_handler(tool, argument, description)


def allow_all_context(session_id: str = "default") -> ToolContext:
    """Return a :class:`ToolContext` that approves every action automatically."""
    return ToolContext(session_id=session_id)


def interactive_context(session_id: str = "default") -> ToolContext:
    """Return a :class:`ToolContext` that prompts on stdout for each action."""

    async def _prompt(tool: str, argument: str, description: str) -> None:
        loop = asyncio.get_event_loop()
        answer = await loop.run_in_executor(
            None,
            input,
            f"\n[opendesk] Allow {tool!r} — {description}? [y/N] ",
        )
        if answer.strip().lower() not in ("y", "yes"):
            raise PermissionDeniedError(
                f"User denied: {tool} — {argument}"
            )

    return ToolContext(session_id=session_id, permission_handler=_prompt)


# ---------------------------------------------------------------------------
# Tool base class
# ---------------------------------------------------------------------------

try:
    from pydantic import BaseModel
except ImportError as _err:
    raise ImportError(
        "pydantic is required: pip install pydantic"
    ) from _err


class Tool(ABC):
    """Abstract base class for all opendesk tools.

    Subclasses must set :attr:`name`, :attr:`description`, and define a nested
    ``Params`` class (a Pydantic model) plus implement :meth:`execute`.

    Example::

        class EchoTool(Tool):
            name = "echo"
            description = "Echo text back."

            class Params(Tool.Params):
                text: str

            async def execute(self, ctx, params):
                return ToolResult(title="Echo", output=params.text)
    """

    name: str
    description: str

    class Params(BaseModel):
        """Default empty parameter model.  Override in subclasses."""

        model_config = {"extra": "forbid"}

    @abstractmethod
    async def execute(self, ctx: ToolContext, params: "Tool.Params") -> ToolResult:
        """Run the tool and return a result."""

    # ------------------------------------------------------------------
    # Schema helpers (used by integration adapters)
    # ------------------------------------------------------------------

    def get_schema(self) -> dict[str, Any]:
        """Return the JSON Schema for this tool's parameters."""
        return self.Params.model_json_schema()

    def to_tool_definition(self) -> dict[str, Any]:
        """Return a generic tool definition dict (name, description, schema)."""
        return {
            "name": self.name,
            "description": self.description,
            "parameters": self.get_schema(),
        }

    def parse_params(self, arguments: dict[str, Any]) -> "Tool.Params":
        """Parse raw argument dict into a typed :class:`Params` instance."""
        return self.Params(**arguments)  # type: ignore[call-arg]

    def __repr__(self) -> str:
        return f"<Tool name={self.name!r}>"
