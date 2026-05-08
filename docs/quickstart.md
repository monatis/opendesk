# Quickstart

> **Requires Python 3.10+**

## Installation

```bash
pip install 'opendesk[core,mcp]'
opendesk install
```

`opendesk install` registers the MCP server with Claude Code globally — no path configuration needed.

---

## 1. Take a screenshot

```python
import asyncio
from opendesk import create_registry, allow_all_context

async def main():
    registry = create_registry()
    ctx = allow_all_context()

    screenshot = registry.get("screenshot")
    result = await screenshot.execute(ctx, screenshot.Params(marks=True))

    # PNG bytes
    png = result.attachments[0].content
    with open("screenshot.png", "wb") as f:
        f.write(png)

    # Text summary of what the model sees
    print(result.output)

asyncio.run(main())
```

---

## 2. Click a button by name

```python
ui = registry.get("ui")

# See what's in the window
result = await ui.execute(ctx, ui.Params(action="get_tree", app="Notepad"))
print(result.output)

# Click a button by its visible label
await ui.execute(ctx, ui.Params(action="click", app="Notepad", title="File"))
```

App names:
- **macOS**: match the name shown in Activity Monitor (e.g. `"Google Chrome"`, `"TextEdit"`)
- **Windows**: match the window title or process name (e.g. `"Notepad"`, `"Chrome"`)
- **Linux**: match the process name (e.g. `"gedit"`, `"firefox"`)

---

## 3. Type text

```python
kb = registry.get("keyboard")
await kb.execute(ctx, kb.Params(action="type", text="Hello, World! 🌍"))
await kb.execute(ctx, kb.Params(action="press", key="enter"))
```

---

## 4. Record and replay a task

```python
# Start recording
learn = registry.get("learn")
await learn.execute(ctx, learn.Params(action="start", task_name="my-task"))

# ... user performs the task ...

# Stop and get trajectory summary
result = await learn.execute(ctx, learn.Params(action="stop"))
print(result.output)  # shows event log + instructions to save

# Save the procedure (after summarizing with an LLM)
await learn.execute(ctx, learn.Params(
    action="save",
    task_name="my-task",
    procedure='{"task_name":"my-task","description":"...","steps":[...],"procedure":"..."}'
))

# Replay later
result = await learn.execute(ctx, learn.Params(action="replay", task_name="my-task"))
print(result.output)  # returns step-by-step instructions for the agent
```

In Claude Code, just say: **"start recording task my-task"** and **"stop recording"** — Claude handles everything automatically.

---

## 5. Full agentic loop with Claude

```python
import anthropic
from opendesk.integrations.claude_code import ClaudeCodeAdapter
from opendesk.registry import create_registry

client = anthropic.Anthropic()
adapter = ClaudeCodeAdapter(create_registry())

messages = [{"role": "user", "content": "Open a text editor, type 'Hello from opendesk', and save the file."}]

result = await adapter.run_loop(
    client=client,
    model="claude-opus-4-6",
    messages=messages,
    system="You are a computer use agent. Use the ui tool first. Mouse is a last resort.",
)
print(result)
```

---

## 6. Permission modes

```python
from opendesk.tools.base import allow_all_context, interactive_context, ToolContext, PermissionDeniedError

# Fully autonomous — approve everything automatically
ctx = allow_all_context()

# Prompt in the terminal before each action
ctx = interactive_context()

# Custom policy
async def my_policy(tool: str, argument: str, description: str) -> None:
    if tool == "app" and "open" in argument.lower():
        raise PermissionDeniedError("App launching is restricted.")

ctx = ToolContext(session_id="safe", permission_handler=my_policy)
```

---

## 7. Restrict the sandbox

```python
from opendesk.computer.sandbox import configure_sandbox
from opendesk.tools.base import ToolContext

# Only allow specific apps, only within a screen region
configure_sandbox(
    session_id="restricted",
    allowed_apps=["Firefox", "Terminal"],
    screen_region=(0, 0, 1280, 800),
)
ctx = ToolContext(session_id="restricted")
```

---

## Next steps

- [Tools reference](tools.md) — full parameter docs for every tool
- [Learn & replay](learn.md) — recording and replaying tasks in depth
- [Integrations](integrations.md) — MCP, OpenAI, LangChain
- [Architecture](architecture.md) — how the layers fit together
