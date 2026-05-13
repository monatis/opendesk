# Quickstart

Pick your language:

| | Python | JavaScript / TypeScript |
|---|---|---|
| Package | `opendesk` (PyPI) | `@vitalops/opendesk-sdk` (npm) |
| Install | `pip install 'opendesk[core,mcp]'` | `npm install @vitalops/opendesk-sdk` |
| MCP register | `opendesk install` | `npx opendesk-js install` |
| Requires | Python 3.10+ | Node.js 18+ |

---

## Python

### Install

```bash
pip install 'opendesk[core,mcp]'
opendesk install
```

`opendesk install` registers the MCP server with Claude Code globally. To remove:

```bash
opendesk uninstall
```

### 1. Take a screenshot

```python
import asyncio
from opendesk import create_registry, allow_all_context

async def main():
    registry = create_registry()
    ctx = allow_all_context()

    screenshot = registry.get("screenshot")
    result = await screenshot.execute(ctx, screenshot.Params(marks=True))

    png = result.attachments[0].content
    with open("screenshot.png", "wb") as f:
        f.write(png)
    print(result.output)

asyncio.run(main())
```

### 2. Click a button by name

```python
ui = registry.get("ui")
await ui.execute(ctx, ui.Params(action="click", app="TextEdit", title="File"))
```

### 3. Type text

```python
kb = registry.get("keyboard")
await kb.execute(ctx, kb.Params(action="type", text="Hello, World! 🌍"))
await kb.execute(ctx, kb.Params(action="press", key="enter"))
```

### 4. Full agentic loop

```python
import anthropic
from opendesk.integrations.claude_code import ClaudeCodeAdapter
from opendesk.registry import create_registry

client = anthropic.Anthropic()
adapter = ClaudeCodeAdapter(create_registry())

messages = [{"role": "user", "content": "Open TextEdit, type 'Hello', save the file."}]
result = await adapter.run_loop(
    client=client,
    model="claude-opus-4-6",
    messages=messages,
    system="You are a computer use agent. Use the ui tool first. Mouse is a last resort.",
)
```

### 5. Permission modes

```python
from opendesk.tools.base import allow_all_context, interactive_context, ToolContext, PermissionDeniedError

ctx = allow_all_context()    # approve everything automatically
ctx = interactive_context()  # prompt in terminal before each action

async def my_policy(tool: str, argument: str, description: str) -> None:
    if tool == "app":
        raise PermissionDeniedError("App launching is restricted.")

ctx = ToolContext(session_id="safe", permission_handler=my_policy)
```

### 6. Restrict the sandbox

```python
from opendesk.computer.sandbox import configure_sandbox
from opendesk.tools.base import ToolContext

configure_sandbox(
    session_id="restricted",
    allowed_apps=["Firefox", "Terminal"],
    screen_region=(0, 0, 1280, 800),
)
ctx = ToolContext(session_id="restricted")
```

---

## JavaScript / TypeScript

No Python required. All desktop automation runs natively in Node.js.

### Install

```bash
npm install @vitalops/opendesk-sdk
```

Register with Claude Code:

```bash
npx opendesk-js install
npx opendesk-js uninstall   # to remove
```

### 1. Take a screenshot

```typescript
import { OpenDeskClient } from "@vitalops/opendesk-sdk";

const client = new OpenDeskClient();
const result = await client.screenshot({ marks: true });
console.log(result.output);
```

### 2. Click a button by name

```typescript
await client.ui({ action: "click", app: "TextEdit", title: "File" });
```

### 3. Type text

```typescript
await client.keyboard({ action: "type", text: "Hello from JS" });
await client.keyboard({ action: "press", key: "enter" });
```

### 4. Full agentic loop (Vercel AI SDK)

```typescript
import { OpenDeskClient } from "@vitalops/opendesk-sdk";
import { generateText } from "ai";
import { anthropic } from "@ai-sdk/anthropic";

const client = new OpenDeskClient();
const shot = await client.screenshot({ marks: true });

const { text } = await generateText({
  model: anthropic("claude-opus-4-6"),
  messages: [
    {
      role: "user",
      content: [
        { type: "text", text: "Click the most prominent button on screen." },
        { type: "image", image: shot.attachments[0].content },
      ],
    },
  ],
});
```

### 5. Native MCP server (Node.js)

```typescript
import { createMcpServer } from "@vitalops/opendesk-sdk";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";

const server = createMcpServer();
await server.connect(new StdioServerTransport());
```

### 6. Custom session or permission handler

```typescript
const client = new OpenDeskClient({
  sessionId: "my-session",
  permissionHandler: async (tool, action, description) => {
    console.log(`Allow: ${description}`);
  },
});
```

---

## Next steps

- [Tools reference](tools.md) — full parameter docs for every tool (same for Python and JS)
- [Integrations](integrations.md) — MCP, OpenAI, LangChain, Vercel AI SDK
- [Architecture](architecture.md) — how the layers fit together
- [JS SDK README](../js/README.md) — JS-specific detail
