# opencua documentation

**Open Computer Use Agent** — a framework for desktop and browser automation that plugs into Claude Code, MCP, OpenAI, and LangChain.

## Contents

| Doc | Description |
|-----|-------------|
| [MCP integration](mcp.md) | How to add opencua to Claude Code, Claude Desktop, Cursor, Continue |
| [Quickstart](quickstart.md) | Installation, first screenshot, agentic loop |
| [Tools reference](tools.md) | All parameters for every tool |
| [Integrations](integrations.md) | Claude Code SDK, OpenAI, LangChain — with full examples |
| [Architecture](architecture.md) | How the layers fit together, how to add custom tools |

## Key concepts

**Tool priority** — always follow this order:
1. `ui` tool — click by element name via the accessibility tree, no coordinates needed.
2. `screenshot` with `marks=True` — get a Set-of-Marks overlay with numbered bounding boxes.
3. `mouse` with `image_width`/`image_height` — last resort for unlabelled canvas areas.

**Set-of-Marks (SoM)** — the screenshot tool draws numbered bounding boxes over every interactive element using data from the platform's native accessibility API (AppleScript / AT-SPI2 / UI Automation). The model can say "click mark 3" instead of guessing pixel coordinates.

**Retina scaling** — on HiDPI displays (Retina Mac, etc.), screenshot pixels ≠ logical pixels. Pass `image_width` and `image_height` from the screenshot result to the mouse tool and coordinates are translated automatically.

**Permission model** — every action goes through `ToolContext.check_permission()` before execution. `allow_all_context()` approves everything; `interactive_context()` prompts on stdout; or inject a custom async callable for your own policy engine.

**Sandbox** — each session has a `ComputerSandbox` that records a full audit log, enforces an app allow-list, and can restrict interactions to a screen region.
