# @vitalops/opendesk-sdk — JavaScript/TypeScript SDK

Give any JavaScript or TypeScript AI agent eyes and hands on your desktop.

No Python required. All desktop automation runs natively in Node.js — screenshot capture, mouse/keyboard control, accessibility APIs, OCR, clipboard, and audit logging.

**Requirements:** Node.js 18+

---

## MCP installation (Claude Code / Claude Desktop)

Install the package and register it as an MCP server in one command:

```bash
npm install @vitalops/opendesk-sdk
npx opendesk-js install
```

The tools (`screenshot`, `mouse`, `keyboard`, `ui`, etc.) are then available in every Claude Code conversation.

To remove:

```bash
npx opendesk-js uninstall
```

### Claude Desktop manual config

Add to `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS):

```json
{
  "mcpServers": {
    "opendesk": {
      "command": "node",
      "args": ["/path/to/node_modules/@vitalops/opendesk-sdk/bin/opendesk-mcp.js"]
    }
  }
}
```

---

## SDK install

```bash
npm install @vitalops/opendesk-sdk
```

---

## Usage

### Programmatic (agent loop)

```typescript
import { OpenDeskClient } from "@vitalops/opendesk-sdk";

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
```

### With Vercel AI SDK

```typescript
import { OpenDeskClient } from "@vitalops/opendesk-sdk";
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
import { createMcpServer } from "@vitalops/opendesk-sdk";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";

const server = createMcpServer();
const transport = new StdioServerTransport();
await server.connect(transport);
```

### Custom permission handler

```typescript
import { OpenDeskClient } from "@vitalops/opendesk-sdk";

const client = new OpenDeskClient({
  sessionId: "my-agent-session",
  permissionHandler: async (tool, action, description) => {
    console.log(`Allow ${description}?`);
    return true;
  },
});
```

---

## Remote machine control

Control another machine on your LAN — same tools, forwarded over an encrypted WebSocket with mutual X25519 authentication and mDNS peer discovery.

```bash
# On the machine to be controlled:
npx opendesk-js pair            # prints a 6-digit pairing code

# On the controlling machine:
npx opendesk-js pair-with <host> <code> --name mini
npx opendesk-js serve           # start the long-running daemon (controlled machine)
```

See [docs/remote-js.md](../docs/remote-js.md) and [docs/protocol.md](../docs/protocol.md) for full details.

### MCP remote tools

Run `npx opendesk-js install` on the controller as normal. Every computer-use tool (`screenshot`, `mouse`, `keyboard`, etc.) accepts an optional `peer` argument to target a remote machine.

**Admin tools** available to the agent:

| Tool | Purpose |
|---|---|
| `opendesk_peers` | List `local` + every trusted peer |
| `opendesk_discover` | Browse the LAN for opendesk peers |
| `opendesk_use <peer>` | Set default peer for subsequent calls (`"local"` to revert) |
| `opendesk_status` | Show current default peer and open connections |
| `opendesk_capabilities [peer]` | Capability manifest of a peer |
| `opendesk_disconnect [peer]` | Close cached connection |

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

---

## How it works

### Local tools

```
Your JS/TS code
      │
      ▼
@vitalops/opendesk-sdk (Node.js)
      │
      ├── screenshot  (screenshot-desktop)
      ├── mouse/keyboard  (@nut-tree-fork/nut-js)
      ├── ui  (osascript / PowerShell UI Automation / xdotool)
      ├── ocr  (tesseract.js)
      └── clipboard  (clipboardy)
```

### Remote tools

```
Controller (your machine)                Controlled machine
      │                                        │
      ▼                                        ▼
connect("mini")           ◄──────────►  opendesk-js serve
      │               ws + ChaCha20-Poly1305    │
      ▼                                        ▼
RemoteComputer                          ToolDispatcher
  .capture()                              maps tool.* RPC → local tools
  .pointer() / .key()                     records every call to audit log
```

All platform-specific automation runs directly in Node.js. No external process is required.

---

## Docs

- [Remote control (JS)](../docs/remote-js.md)
- [Protocol](../docs/protocol.md)
- [Tools reference](../docs/tools.md)
- [Architecture](../docs/architecture.md)

---

## License

MIT
