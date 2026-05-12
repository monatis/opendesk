# @opendesk/sdk — JavaScript/TypeScript SDK

Give any JavaScript or TypeScript AI agent eyes and hands on your desktop.

No Python required. All desktop automation runs natively in Node.js — screenshot capture, mouse/keyboard control, accessibility APIs, OCR, clipboard, and audit logging.

---

## Requirements

- Node.js 18+

---

## Install

```bash
npm install @opendesk/sdk
```

### Claude Code / Claude Desktop

```bash
npx opendesk-js install
```

This registers the native MCP server with Claude Code. The tools (`screenshot`, `mouse`, `keyboard`, `ui`, etc.) are then available in every Claude Code conversation.

To remove:

```bash
npx opendesk-js uninstall
```

### Claude Desktop

Add to your config file (`~/Library/Application Support/Claude/claude_desktop_config.json` on macOS):

```json
{
  "mcpServers": {
    "opendesk": {
      "command": "node",
      "args": ["/path/to/node_modules/@opendesk/sdk/bin/opendesk-mcp.js"]
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
```

### With Vercel AI SDK

```typescript
import { OpenDeskClient } from "@opendesk/sdk";
import { generateText } from "ai";
import { anthropic } from "@ai-sdk/anthropic";

const client = new OpenDeskClient();

const shot = await client.screenshot({ marks: true });
const response = await generateText({
  model: anthropic("claude-opus-4-6"),
  messages: [
    {
      role: "user",
      content: [
        { type: "text", text: "What do you see on screen? Click the most prominent button." },
        { type: "image", image: shot.attachments[0].content },
      ],
    },
  ],
});
```

### Expose as MCP server

```typescript
import { createMcpServer } from "@opendesk/sdk";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";

const server = createMcpServer();
const transport = new StdioServerTransport();
await server.connect(transport);
```

### Custom session ID or permission handler

```typescript
import { OpenDeskClient } from "@opendesk/sdk";

const client = new OpenDeskClient({
  sessionId: "my-agent-session",
  permissionHandler: async (tool, action, description) => {
    console.log(`Allow ${description}? [y/n]`);
    // return true to allow, false to deny
    return true;
  },
});
```

---

## Tools

Full reference: [docs/tools.md](../docs/tools.md)

| Tool | Method | Description |
|------|--------|-------------|
| `screenshot` | `client.screenshot(params?)` | Capture screen, optional SoM marks |
| `ui` | `client.ui(params)` | Click/type by element name — no coordinates |
| `mouse` | `client.mouse(params)` | Pixel-level mouse control |
| `keyboard` | `client.keyboard(params)` | Type, press keys, hotkeys |
| `app` | `client.app(params)` | Open, close, focus applications |
| `clipboard` | `client.clipboard(params)` | Read/write system clipboard |
| `ocr` | `client.ocr(params?)` | Extract text from screen |
| `audit` | `client.audit(params?)` | Show session audit log |

---

## How it works

```
Your JS/TS code
      │
      ▼
@opendesk/sdk (Node.js)
      │
      ├── screenshot  (screenshot-desktop)
      ├── mouse/keyboard  (@nut-tree-fork/nut-js)
      ├── ui  (osascript / PowerShell UI Automation / xdotool)
      ├── ocr  (tesseract.js)
      ├── clipboard  (clipboardy)
      └── audit  (in-process session log)
```

All platform-specific automation runs directly in Node.js. No external process is required.

---

## License

MIT
