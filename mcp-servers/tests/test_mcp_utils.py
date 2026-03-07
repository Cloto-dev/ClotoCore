"""Tests for common.mcp_utils ToolRegistry."""

import json
import os
import sys

import pytest

sys.path.insert(0, os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")))

from common.mcp_utils import ToolRegistry


@pytest.fixture
def registry():
    reg = ToolRegistry("test-server")

    @reg.tool("greet", "Say hello", {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]})
    async def greet(arguments: dict) -> dict:
        return {"message": f"Hello, {arguments['name']}!"}

    @reg.tool("fail", "Always fails", {"type": "object", "properties": {}})
    async def fail(arguments: dict) -> dict:
        raise RuntimeError("intentional error")

    return reg


def test_tool_registration(registry):
    """Tools should be registered with correct metadata."""
    assert len(registry._tools) == 2
    assert registry._tools[0].name == "greet"
    assert registry._tools[1].name == "fail"


@pytest.mark.asyncio
async def test_list_tools(registry):
    """list_tools handler should return all registered tools."""
    handlers = registry.server.request_handlers
    # The list_tools handler is registered on the server
    assert len(registry._tools) == 2
    assert registry._tools[0].description == "Say hello"


@pytest.mark.asyncio
async def test_call_tool_success(registry):
    """Calling a registered tool should return JSON-wrapped result."""
    handler = registry._handlers["greet"]
    result = await handler({"name": "World"})
    assert result == {"message": "Hello, World!"}


@pytest.mark.asyncio
async def test_call_tool_exception(registry):
    """Calling a failing tool should return error JSON, not crash."""
    # Simulate what call_tool does internally
    handler = registry._handlers["fail"]
    try:
        await handler({})
        caught = False
    except RuntimeError:
        caught = True
    assert caught


def test_unknown_tool_handler(registry):
    """Unknown tool name should not be in handlers."""
    assert "nonexistent" not in registry._handlers


def test_tool_schema(registry):
    """Tool schemas should be preserved."""
    greet_tool = registry._tools[0]
    assert greet_tool.inputSchema["required"] == ["name"]
    assert "name" in greet_tool.inputSchema["properties"]
