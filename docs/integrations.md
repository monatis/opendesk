# Integrations

opendesk tools can be used with any agentic harness. This page documents each integration in detail.

---

## MCP (Model Context Protocol)

MCP is the recommended integration. Any MCP-compatible client automatically gets all opendesk tools — no custom glue code needed.

### Quickstart

```bash
# Install
pip install 'opendesk[core,mcp]'

# Run server (stdio transport)
opendesk-mcp
```

### Claude Desktop

Add to `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "opendesk": {
      "command": "opendesk-mcp"
    }
  }
}
```

Restart Claude Desktop. All opendesk tools appear in the tool palette.

### Continue (VS Code)

Add to `.continue/config.json`:

```json
{
  "mcpServers": [
    {
      "name": "opendesk",
      "command": "opendesk-mcp",
      "transport": "stdio"
    }
  ]
}
```

### Cursor

Add to `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "opendesk": {
      "command": "opendesk-mcp"
    }
  }
}
```

### In-process MCP server

```python
import asyncio
from mcp.server.stdio import stdio_server
from opendesk.integrations.mcp import create_mcp_server
from opendesk.registry import create_registry
from opendesk.tools.base import interactive_context

# Use interactive_context to prompt before each action
server = create_mcp_server(
    registry=create_registry(),
    ctx=interactive_context(),
)

async def main():
    async with stdio_server() as (r, w):
        await server.run(r, w, server.create_initialization_options())

asyncio.run(main())
```

### Custom permission handler with MCP

```python
from opendesk.tools.base import ToolContext, PermissionDeniedError

BLOCKED_APPS = {"System Preferences", "Keychain Access"}

async def policy(tool: str, argument: str, description: str) -> None:
    for blocked in BLOCKED_APPS:
        if blocked.lower() in argument.lower():
            raise PermissionDeniedError(f"Blocked: {blocked}")

ctx = ToolContext(session_id="mcp", permission_handler=policy)
server = create_mcp_server(ctx=ctx)
```

---

## Claude Code / Anthropic SDK

### Agentic loop (recommended)

```python
import anthropic
from opendesk.integrations.claude_code import ClaudeCodeAdapter
from opendesk.registry import create_registry
from opendesk.tools.base import allow_all_context

client = anthropic.Anthropic()
adapter = ClaudeCodeAdapter(
    registry=create_registry(),
    ctx=allow_all_context(),
)

messages = [{"role": "user", "content": "Open TextEdit and type 'Hello World'"}]

result = await adapter.run_loop(
    client=client,
    model="claude-opus-4-6",
    messages=messages,
    system=(
        "You are a computer use agent. "
        "Always use the ui tool first. "
        "Use the mouse tool only for unlabelled canvas areas."
    ),
    max_tokens=8192,
    max_iterations=20,
)
print(result)
```

### Manual control

```python
response = client.messages.create(
    model="claude-opus-4-6",
    max_tokens=4096,
    tools=adapter.tool_definitions(),
    messages=messages,
)

messages.append({"role": "assistant", "content": response.content})

if response.stop_reason == "tool_use":
    # Executes all tool_use blocks in parallel
    tool_results = await adapter.handle_response(response)
    messages.append({"role": "user", "content": tool_results})
```

### Tool definitions format

```python
adapter.tool_definitions()
# Returns:
# [
#   {
#     "name": "screenshot",
#     "description": "...",
#     "input_schema": { "type": "object", "properties": {...} }
#   },
#   ...
# ]
```

---

## OpenAI function calling

Works with OpenAI API and any OpenAI-compatible endpoint (Groq, Together AI, Ollama, LiteLLM, vLLM, Fireworks, etc.).

### Agentic loop

```python
from openai import OpenAI
from opendesk.integrations.openai_compat import OpenAIAdapter
from opendesk.registry import create_registry

client = OpenAI()
adapter = OpenAIAdapter(create_registry())

messages = [{"role": "user", "content": "Take a screenshot and describe what you see"}]
result = await adapter.run_loop(client, model="gpt-4o", messages=messages)
```

### With Groq

```python
from groq import Groq
client = Groq()
result = await adapter.run_loop(client, model="llama-3.3-70b-versatile", messages=messages)
```

### With Ollama

```python
from openai import OpenAI
client = OpenAI(base_url="http://localhost:11434/v1", api_key="ollama")
result = await adapter.run_loop(client, model="qwen2.5:72b", messages=messages)
```

### Manual control

```python
from openai import OpenAI
from opendesk.integrations.openai_compat import OpenAIAdapter

client = OpenAI()
adapter = OpenAIAdapter()

response = client.chat.completions.create(
    model="gpt-4o",
    tools=adapter.tool_definitions(),
    messages=messages,
)

choice = response.choices[0]
messages.append(choice.message)

if choice.finish_reason == "tool_calls":
    tool_messages = await adapter.handle_response(choice.message)
    messages.extend(tool_messages)
```

---

## LangChain / LangGraph

```python
from langchain_anthropic import ChatAnthropic
from langgraph.prebuilt import create_react_agent

from opendesk.integrations.langchain_compat import as_langchain_tools
from opendesk.registry import create_registry

tools = as_langchain_tools(create_registry())
llm = ChatAnthropic(model="claude-opus-4-6")
agent = create_react_agent(llm, tools)

result = agent.invoke({"messages": [("user", "Open Safari and take a screenshot")]})
print(result["messages"][-1].content)
```

### With custom context

```python
from opendesk.tools.base import ToolContext, PermissionDeniedError

async def strict_policy(tool: str, argument: str, description: str) -> None:
    if tool == "app" and "open" in argument:
        raise PermissionDeniedError("App launching not allowed in this session.")

ctx = ToolContext(session_id="langchain", permission_handler=strict_policy)
tools = as_langchain_tools(create_registry(), ctx=ctx)
```

---

## Custom / generic integration

If your framework isn't listed above, use the raw tool API directly:

```python
import json
from opendesk.registry import create_registry
from opendesk.tools.base import allow_all_context

registry = create_registry()
ctx = allow_all_context()

# Get JSON schemas for all tools (to pass to your LLM)
tool_schemas = [
    {
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.get_schema(),
    }
    for tool in registry.tools()
]

# Execute a tool from LLM output
tool_name = "screenshot"       # from LLM response
arguments = {"marks": True}    # from LLM response

tool = registry.get(tool_name)
params = tool.parse_params(arguments)
result = await tool.execute(ctx, params)

print(result.output)
for att in result.attachments:
    print(f"Attachment: {att.filename} ({att.media_type}, {len(att.content)} bytes)")
```

---

## Sandbox configuration

Restrict what the agent can do in a session:

```python
from opendesk.computer.sandbox import configure_sandbox, get_sandbox
from opendesk.tools.base import ToolContext

# Only allow specific apps and a screen region
configure_sandbox(
    session_id="restricted",
    allowed_apps=["Safari", "Terminal"],
    screen_region=(0, 0, 1280, 800),  # (x, y, width, height)
)

ctx = ToolContext(session_id="restricted")
# Now the agent can only screenshot/click within x:0–1280, y:0–800
# and can only open/focus Safari or Terminal.

# Audit everything that happened
sandbox = get_sandbox("restricted")
print(sandbox.summary())
log = sandbox.export_audit_log()  # list of dicts
```
