# Quickstart

## Installation

```bash
pip install 'opencua[core,mcp]'
```

## 1. Take a screenshot

```python
import asyncio
from opencua import create_registry, allow_all_context

async def main():
    registry = create_registry()
    ctx = allow_all_context()

    screenshot = registry.get("screenshot")
    result = await screenshot.execute(ctx, screenshot.Params(marks=True, show_cursor=True))

    # PNG bytes
    png = result.attachments[0].content
    with open("/tmp/screenshot.png", "wb") as f:
        f.write(png)

    # Text summary of what the model sees
    print(result.output)

asyncio.run(main())
```

## 2. Click a button by name

```python
ui = registry.get("ui")

# See what's in the window
result = await ui.execute(ctx, ui.Params(action="get_tree", app="TextEdit"))
print(result.output)

# Click the Save button
await ui.execute(ctx, ui.Params(action="click", app="TextEdit", title="Save"))
```

## 3. Type text

```python
kb = registry.get("keyboard")
await kb.execute(ctx, kb.Params(action="type", text="Hello, World! 🌍"))
await kb.execute(ctx, kb.Params(action="press", key="enter"))
```

## 4. Use as an MCP server

```bash
# Run once, then connect from Claude Desktop / Continue / Cursor
opencua-mcp
```

Add to your MCP client's config:
```json
{ "command": "opencua-mcp" }
```

## 5. Full agentic loop with Claude

```python
import anthropic
from opencua.integrations.claude_code import ClaudeCodeAdapter
from opencua.registry import create_registry

client = anthropic.Anthropic()
adapter = ClaudeCodeAdapter(create_registry())

messages = [{"role": "user", "content": "Open TextEdit, type 'Hello from opencua', and save the file."}]

result = await adapter.run_loop(
    client=client,
    model="claude-opus-4-6",
    messages=messages,
    system="You are a computer use agent. Use the ui tool first. Mouse is a last resort.",
)
print(result)
```

## 6. Permission modes

```python
from opencua.tools.base import allow_all_context, interactive_context, ToolContext, PermissionDeniedError

# Fully autonomous (approve everything)
ctx = allow_all_context()

# Prompt on stdout before each action
ctx = interactive_context()

# Custom policy
async def my_policy(tool: str, argument: str, description: str) -> None:
    if tool == "app" and "open" in argument.lower():
        raise PermissionDeniedError("App launching is restricted.")

ctx = ToolContext(session_id="safe", permission_handler=my_policy)
```

## 7. Restrict the sandbox

```python
from opencua.computer.sandbox import configure_sandbox

# Only allow Safari and Terminal, only within the left half of a 2560px screen
configure_sandbox(
    session_id="restricted",
    allowed_apps=["Safari", "Terminal"],
    screen_region=(0, 0, 1280, 1600),
)
ctx = ToolContext(session_id="restricted")
```

## Next steps

- [Tools reference](tools.md) — full parameter docs for every tool
- [Integrations](integrations.md) — MCP, OpenAI, LangChain deep dive
- [Architecture](architecture.md) — how the layers fit together
