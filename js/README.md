# @opendesk/sdk — JavaScript/TypeScript SDK

Give any JavaScript or TypeScript AI agent eyes and hands on your desktop.

The JS SDK is a typed bridge to the Python `opendesk-mcp` server. All desktop automation runs in the Python layer — the JS SDK gives you full type safety and a clean API for use in Node.js agent loops.

---

## Requirements

- Node.js 18+
- Python `opendesk` installed: `pip install 'opendesk[core,mcp]'`

---

## Install

```bash
npm install @opendesk/sdk
```

### Claude Code / Claude Desktop

```bash
npx opendesk-js install
```

This registers the JS MCP bridge with Claude Code. From that point the same tools (`screenshot`, `mouse`, `keyboard`, `ui`, etc.) are available in every Claude Code conversation.

To remove:

```bash
npx opendesk-js uninstall
```

### Claude Desktop

Add to your config file:

```json
{
  "mcpServers": {
    "opendesk": {
      "command": "node",
      "args": ["/path/to/node_modules/@opendesk/sdk/bin/opendesk-mcp-bridge.js"]
    }
  }
}
```

---

## Usage

### Programmatic (agent loop)

```typescript
import { OpenDeskClient } from "@opendesk/sdk";

const client = new OpenDeskClient();
await client.connect();

// Take a screenshot with Set-of-Marks
const shot = await client.screenshot({ marks: true });
console.log(shot.output);

// Click a button by name — no coordinates needed
await client.ui({ action: "click", app: "Safari", title: "Go" });

// Type text
await client.keyboard({ action: "type", text: "Hello from JS" });

// Open an app
await client.app({ action: "open", name: "Spotify" });

// Read clipboard
const clip = await client.clipboard({ action: "read" });
console.log(clip.output);

// OCR a region
const text = await client.ocr({ region: [0, 0, 800, 400] });

// Show audit log
const log = await client.audit({ format: "summary" });

await client.disconnect();
```

### With Vercel AI SDK

```typescript
import { OpenDeskClient } from "@opendesk/sdk";
import { generateText } from "ai";
import { anthropic } from "@ai-sdk/anthropic";

const client = new OpenDeskClient();
await client.connect();

// Use opendesk tools inside your agent loop
const shot = await client.screenshot({ marks: true });
const response = await generateText({
  model: anthropic("claude-opus-4-6"),
  messages: [
    {
      role: "user",
      content: [
        { type: "text", text: "What do you see on screen? Click the most prominent button." },
        { type: "image", image: Buffer.from(shot.attachments[0].contentBase64, "base64") },
      ],
    },
  ],
});

await client.disconnect();
```

### MCP bridge server

If you want to serve opendesk tools from a Node.js process:

```typescript
import { createMcpBridge } from "@opendesk/sdk";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";

const server = await createMcpBridge();
const transport = new StdioServerTransport();
await server.connect(transport);
```

### Custom Python server path

```typescript
const client = new OpenDeskClient({
  command: "/path/to/venv/bin/opendesk-mcp",
});
```

---

## Tools

All tools mirror the Python SDK exactly. Full reference: [docs/tools.md](../docs/tools.md)

| Tool | Method | Description |
|------|--------|-------------|
| `screenshot` | `client.screenshot(params?)` | Capture screen, optional SoM marks |
| `ui` | `client.ui(params)` | Click/type by element name — no coordinates |
| `mouse` | `client.mouse(params)` | Pixel-level mouse control |
| `keyboard` | `client.keyboard(params)` | Type, press keys, hotkeys |
| `app` | `client.app(params)` | Open, close, focus applications |
| `clipboard` | `client.clipboard(params)` | Read/write system clipboard |
| `ocr` | `client.ocr(params?)` | Extract text from screen |
| `learn` | `client.learn(params)` | Record and replay workflows |
| `schedule` | `client.schedule(params)` | Schedule computer-use tasks |
| `audit` | `client.audit(params?)` | Show session audit log |

---

## How it works

```
Your JS/TS code
      │
      │  MCP protocol over stdio
      ▼
@opendesk/sdk (Node.js)
      │
      │  spawns + communicates via stdio
      ▼
opendesk-mcp (Python process)
      │
      ├── screenshot, OCR, SoM marks
      ├── mouse, keyboard (pyautogui)
      ├── accessibility API (AppleScript / AT-SPI2 / UI Automation)
      └── learn, schedule, audit
```

The Python process owns all platform-specific desktop automation. The JS SDK is a typed MCP client — no platform code lives in JS.

---

## License

MIT
