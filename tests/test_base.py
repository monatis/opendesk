"""Tests for the framework core — no hardware dependencies."""

from __future__ import annotations

import asyncio
import pytest
from unittest.mock import AsyncMock, patch

from opencua.tools.base import (
    Attachment,
    PermissionDeniedError,
    Tool,
    ToolContext,
    ToolResult,
    allow_all_context,
)
from opencua.registry import ToolRegistry, create_registry, create_minimal_registry


# ---------------------------------------------------------------------------
# ToolResult
# ---------------------------------------------------------------------------

class TestToolResult:
    def test_to_dict_excludes_attachments(self):
        r = ToolResult(title="T", output="O", error=False)
        d = r.to_dict()
        assert d["title"] == "T"
        assert d["output"] == "O"
        assert "attachments" not in d

    def test_to_dict_b64_includes_attachments(self):
        att = Attachment("img.png", b"\x89PNG", "image/png")
        r = ToolResult(title="T", output="O", attachments=[att])
        d = r.to_dict_b64()
        assert len(d["attachments"]) == 1
        assert d["attachments"][0]["filename"] == "img.png"
        assert d["attachments"][0]["content_b64"]  # non-empty base64


# ---------------------------------------------------------------------------
# Attachment
# ---------------------------------------------------------------------------

class TestAttachment:
    def test_to_base64_roundtrip(self):
        import base64
        data = b"hello world"
        att = Attachment("file.txt", data, "text/plain")
        decoded = base64.b64decode(att.to_base64())
        assert decoded == data


# ---------------------------------------------------------------------------
# ToolContext
# ---------------------------------------------------------------------------

class TestToolContext:
    @pytest.mark.asyncio
    async def test_allow_all_permits_everything(self):
        ctx = allow_all_context()
        # Should not raise
        await ctx.check_permission("screenshot", "capture screen", "Take screenshot")

    @pytest.mark.asyncio
    async def test_custom_handler_can_deny(self):
        async def deny_all(tool, argument, description):
            raise PermissionDeniedError("denied")

        ctx = ToolContext(session_id="test", permission_handler=deny_all)
        with pytest.raises(PermissionDeniedError):
            await ctx.check_permission("mouse", "click at (0,0)", "Click")

    @pytest.mark.asyncio
    async def test_custom_handler_can_allow(self):
        called = []

        async def record(tool, argument, description):
            called.append((tool, argument))

        ctx = ToolContext(session_id="test", permission_handler=record)
        await ctx.check_permission("screenshot", "capture", "Take screenshot")
        assert called == [("screenshot", "capture")]


# ---------------------------------------------------------------------------
# Tool ABC
# ---------------------------------------------------------------------------

class _EchoTool(Tool):
    name = "echo"
    description = "Echo text."

    class Params(Tool.Params):
        text: str

    async def execute(self, ctx, params):
        return ToolResult(title="Echo", output=params.text)


class TestTool:
    def test_get_schema_has_properties(self):
        tool = _EchoTool()
        schema = tool.get_schema()
        assert "properties" in schema
        assert "text" in schema["properties"]

    def test_parse_params_validates(self):
        tool = _EchoTool()
        p = tool.parse_params({"text": "hello"})
        assert p.text == "hello"

    def test_parse_params_rejects_unknown(self):
        from pydantic import ValidationError
        tool = _EchoTool()
        with pytest.raises(ValidationError):
            tool.parse_params({"text": "hi", "extra_field": "oops"})

    def test_to_tool_definition(self):
        tool = _EchoTool()
        d = tool.to_tool_definition()
        assert d["name"] == "echo"
        assert d["description"] == "Echo text."
        assert "parameters" in d

    @pytest.mark.asyncio
    async def test_execute(self):
        tool = _EchoTool()
        ctx = allow_all_context()
        result = await tool.execute(ctx, _EchoTool.Params(text="hello"))
        assert result.output == "hello"
        assert not result.error


# ---------------------------------------------------------------------------
# ToolRegistry
# ---------------------------------------------------------------------------

class TestToolRegistry:
    def test_register_and_get(self):
        registry = ToolRegistry()
        tool = _EchoTool()
        registry.register(tool)
        assert registry.get("echo") is tool

    def test_get_missing_raises(self):
        registry = ToolRegistry()
        with pytest.raises(KeyError, match="echo"):
            registry.get("echo")

    def test_names_sorted(self):
        registry = ToolRegistry()
        registry.register(_EchoTool())

        class _ZTool(Tool):
            name = "zzz"
            description = "Z"
            async def execute(self, ctx, params): ...

        registry.register(_ZTool())
        assert registry.names() == ["echo", "zzz"]

    def test_contains(self):
        registry = ToolRegistry()
        registry.register(_EchoTool())
        assert "echo" in registry
        assert "missing" not in registry

    def test_len(self):
        registry = ToolRegistry()
        assert len(registry) == 0
        registry.register(_EchoTool())
        assert len(registry) == 1


# ---------------------------------------------------------------------------
# create_registry
# ---------------------------------------------------------------------------

class TestCreateRegistry:
    def test_all_tools_present(self):
        registry = create_registry()
        expected = {"screenshot", "mouse", "keyboard", "app", "ui", "clipboard", "ocr"}
        assert expected.issubset(set(registry.names()))

    def test_minimal_registry(self):
        registry = create_minimal_registry()
        assert "screenshot" in registry
        assert "ocr" not in registry

    def test_tools_have_schemas(self):
        registry = create_registry()
        for tool in registry.tools():
            schema = tool.get_schema()
            assert "properties" in schema or schema.get("type") == "object"
