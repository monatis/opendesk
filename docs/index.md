# opendesk documentation

**Open Computer Use Agent** — gives any AI agent eyes and hands on your desktop. Works on macOS, Linux, and Windows.

## Contents

| Doc | Description |
|-----|-------------|
| [Quickstart](quickstart.md) | Install, first screenshot, agentic loop |
| [MCP integration](mcp.md) | Add opendesk to Claude Code, Claude Desktop, Cursor, Continue |
| [Automation](automation.md) | Record, replay, and schedule desktop tasks |
| [Tools reference](tools.md) | Full parameter docs for every tool |
| [Integrations](integrations.md) | Anthropic SDK, OpenAI, LangChain |
| [Architecture](architecture.md) | How the layers fit together, how to add custom tools |

## Key concepts

**Tool priority** — always follow this order:
1. `ui` — click by element name via the accessibility tree, no coordinates needed.
2. `screenshot` with `marks=True` — get a Set-of-Marks overlay with numbered bounding boxes.
3. `mouse` with `image_width`/`image_height` — last resort for unlabelled canvas areas.

**Set-of-Marks (SoM)** — the screenshot tool draws numbered bounding boxes over every interactive element using data from the platform's native accessibility API (AppleScript on macOS, AT-SPI2 on Linux, UI Automation on Windows). The model can say "click mark 3" instead of guessing pixel coordinates.

**HiDPI scaling** — on high-resolution displays (Retina, 4K), screenshot pixels ≠ logical pixels. Pass `image_width` and `image_height` from the screenshot result to the mouse tool and coordinates are translated automatically.

**Learn & replay** — the `learn` tool records any desktop workflow (mouse, keyboard, screenshots) and summarizes it into a reusable procedure. Replay it later and the agent re-executes the steps using the current screen state — no hardcoded coordinates or paths.

**Permission model** — every action goes through `ToolContext.check_permission()` before execution. `allow_all_context()` approves everything; `interactive_context()` prompts in the terminal; or inject a custom async callable for your own policy engine.

**Sandbox** — each session has a `ComputerSandbox` that records a full audit log, enforces an app allow-list, and can restrict interactions to a screen region.
