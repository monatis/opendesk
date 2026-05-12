# opendesk — Python SDK

Give any AI agent eyes and hands on your desktop.

**macOS · Linux · Windows**

[![PyPI](https://img.shields.io/pypi/v/opendesk)](https://pypi.org/project/opendesk/)
[![Python](https://img.shields.io/pypi/pyversions/opendesk)](https://pypi.org/project/opendesk/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](../LICENSE)

---

## Install

```bash
pip install 'opendesk[core,mcp]'
opendesk install
```

`opendesk install` registers the MCP server with Claude Code globally.

---

## Quick start

```python
import asyncio
from opendesk import create_registry, allow_all_context

async def main():
    registry = create_registry()
    ctx = allow_all_context()

    # Screenshot with Set-of-Marks
    shot = registry.get("screenshot")
    result = await shot.execute(ctx, shot.Params(marks=True))
    print(result.output)

    # Click a button by name — no coordinates needed
    ui = registry.get("ui")
    await ui.execute(ctx, ui.Params(action="click", app="TextEdit", title="File"))

asyncio.run(main())
```

---

## Installation options

```bash
pip install opendesk                              # core framework only
pip install 'opendesk[core,mcp]'                  # + screen capture + MCP server (recommended)
pip install 'opendesk[core,mcp,learn]'            # + task recording and replay
pip install 'opendesk[core,mcp,learn,schedule]'   # + scheduled tasks
pip install 'opendesk[all]'                       # everything
```

---

## Tools

| Tool | What it does |
|------|-------------|
| `screenshot` | Capture screen with Set-of-Marks on every interactive element |
| `ui` | Click and type by element name — no coordinates needed |
| `mouse` | Pixel-level mouse control for anything `ui` can't reach |
| `keyboard` | Type text, press keys, send hotkeys |
| `app` | Open, close, and focus applications |
| `clipboard` | Read and write the system clipboard |
| `ocr` | Extract text from any region of the screen |
| `learn` | Record a workflow once, replay it anytime |
| `schedule` | Run any task on a timer |
| `audit` | Show the session audit log in any MCP session |

Full reference: [docs/tools.md](../docs/tools.md)

---

## MCP integrations

### Claude Code

```bash
opendesk install        # register globally
opendesk uninstall      # remove
```

### Claude Desktop

```json
{
  "mcpServers": {
    "opendesk": { "command": "opendesk-mcp" }
  }
}
```

### Cursor / Continue

```json
{
  "mcpServers": [{ "name": "opendesk", "command": "opendesk-mcp", "transport": "stdio" }]
}
```

---

## Agent integrations

### Anthropic SDK

```python
import anthropic
from opendesk.integrations.claude_code import ClaudeCodeAdapter
from opendesk.registry import create_registry

client = anthropic.Anthropic()
adapter = ClaudeCodeAdapter(create_registry())

result = await adapter.run_loop(
    client=client,
    model="claude-opus-4-6",
    messages=[{"role": "user", "content": "Open TextEdit and type Hello."}],
    system="Use the ui tool first. Mouse is a last resort.",
)
```

### OpenAI / on-device models (Ollama, vLLM, llama.cpp)

```python
from openai import OpenAI
from opendesk.integrations.openai_compat import OpenAIAdapter

# Any OpenAI-compatible endpoint works
client = OpenAI(base_url="http://localhost:11434/v1", api_key="ollama")
adapter = OpenAIAdapter()
result = await adapter.run_loop(client, model="qwen2.5:72b", messages=messages)
```

### LangChain

```python
from langchain_anthropic import ChatAnthropic
from langgraph.prebuilt import create_react_agent
from opendesk.integrations.langchain_compat import as_langchain_tools
from opendesk.registry import create_registry

tools = as_langchain_tools(create_registry())
agent = create_react_agent(ChatAnthropic(model="claude-opus-4-6"), tools)
```

---

## Build from source

```bash
cd python
pip install -e '.[core,mcp]'
```

---

## Docs

- [Quickstart](../docs/quickstart.md)
- [Tools reference](../docs/tools.md)
- [Integrations](../docs/integrations.md)
- [Architecture](../docs/architecture.md)

## License

MIT
