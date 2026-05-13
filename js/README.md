# @vitalops/opendesk-sdk — JavaScript/TypeScript SDK

Give any JavaScript or TypeScript AI agent eyes and hands on your desktop.

No Python required. All desktop automation runs natively in Node.js — screenshot capture, mouse/keyboard control, accessibility APIs, OCR, clipboard, and audit logging.

---

## Requirements

- Node.js 18+

---

## Install

```bash
npm install @vitalops/opendesk-sdk
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
      "args": ["/path/to/node_modules/@vitalops/opendesk-sdk/bin/opendesk-mcp.js"]
    }
  }
}
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

// Show audit log
const log = await client.audit({ format: "summary" });
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

### Custom session ID or permission handler

```typescript
import { OpenDeskClient } from "@vitalops/opendesk-sdk";

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

## Remote machine control

Control another machine on your LAN from your agent — screenshot, mouse, keyboard, all the same tools, just forwarded over an encrypted WebSocket.

### Install with remote deps

```bash
npm install @vitalops/opendesk-sdk
# ws, msgpackr, and bonjour-service are bundled as regular deps
```

### One-time pairing

**On the controlled machine** (run `opendesk serve` — Python or JS):

```bash
# Python
pip install 'opendesk[remote]'
opendesk pair

# or JS
npx opendesk-js pair
```

You'll see a 6-digit pairing code. Leave the terminal open.

**On the controller (JS)**:

```bash
npx opendesk-js pair-with 192.168.1.42 428901 --name mini
# Paired with mini (9c2f:...)
```

### Start the daemon on the controlled machine

```bash
# Python
opendesk serve

# or JS
npx opendesk-js serve
```

### Connect from the controller

```typescript
import { connect } from "@vitalops/opendesk-sdk";

const remote = await connect("mini");           // peer name from pairing
const shot = await remote.capture();
console.log(shot.width, shot.height);
await remote.close();
```

Or discover peers on the LAN first:

```typescript
import { discover, connect } from "@vitalops/opendesk-sdk";

const peers = await discover(2000);             // browse for 2 s
console.log(peers);                             // [{ name, host, port, fingerprint }]

const remote = await connect(peers[0]);         // connect by DiscoveredPeer object
```

### Serve (JS daemon)

```typescript
import { OpendeskServer } from "@vitalops/opendesk-sdk";
import { Identity } from "@vitalops/opendesk-sdk";
import { TrustedPeers } from "@vitalops/opendesk-sdk";

const identity = Identity.loadOrCreate();
const trusted = new TrustedPeers();
const server = new OpendeskServer(identity, trusted);
await server.start();
console.log("Listening on port", server.port);
```

### MCP remote tools

Run the MCP server as normal. Every computer-use tool (`screenshot`, `mouse`, `keyboard`, etc.) accepts an optional `peer` argument to target a remote machine:

```
opendesk_use mini          # set default peer for this session
screenshot                 # taken on mini
mouse { action: "move", x: 100, y: 200 }  # on mini
opendesk_use local         # switch back to local
```

**Admin tools** available to the agent:

| Tool | Purpose |
|---|---|
| `opendesk_peers` | List trusted peers |
| `opendesk_discover` | Browse LAN for opendesk peers |
| `opendesk_use <peer>` | Set default peer (`"local"` to revert) |
| `opendesk_status` | Show current default peer and connections |
| `opendesk_capabilities [peer]` | Capability manifest |
| `opendesk_disconnect [peer]` | Close cached connection |

### JS CLI commands

```
npx opendesk-js pair           [--port N] [--code XXXXXX] [--timeout S]
npx opendesk-js serve          [--port N] [--host H] [--no-mdns]
npx opendesk-js pair-with HOST CODE [--port N] [--name NAME]
npx opendesk-js discover       [--timeout S]
npx opendesk-js peers          [list | default [NAME] | rename NAME NEW | remove NAME]
npx opendesk-js connect PEER
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
@vitalops/opendesk-sdk (Node.js)
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
