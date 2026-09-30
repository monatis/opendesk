# MCP Apps (Interactive UI Viewport)

OpenDesk supports the **MCP Apps (MCP UI) extension specification** (SEP-1865: `io.modelcontextprotocol/ui`).

Instead of only returning static base64 images that scroll through chat history, OpenDesk can return an **interactive, sandboxed remote desktop viewport** embedded directly inside supported MCP hosts (such as Claude Desktop, Goose, and custom agent harnesses like `isanagent`).

---

## Capabilities & Advantages

- **Zero-Token Human Observation**: The user can observe and interact with the remote desktop live inside the conversation without loading high-resolution screenshots into the LLM context window.
- **Human-in-the-Loop Takeover**: If the agent encounters a CAPTCHA, MFA prompt, or credential modal, the user can click and type directly inside the embedded canvas.
- **Active Peer Switching**: Change target peers (between `local` and paired remote machines) using the inline peer selector.
- **Theme Adaptation**: Automatically adapts to host themes (dark/light) via `ui/notifications/host-context-changed`.
- **Auto-Refresh & Manual Controls**: Provides configurable auto-refresh rates (1.5s, 3s, 5s, or manual capture), mouse mode selectors (Left click, Right click, Double click), text typing, and special key combinations (`Enter`, `Esc`, `Tab`, `Win`, `Ctrl+C`, `Ctrl+V`, `Alt+Tab`).

---

## MCP Protocol Details

### 1. Resources
OpenDesk advertises and serves the following MCP App resource:

- **URI**: `ui://opendesk/viewport`
- **MIME Type**: `text/html;profile=mcp-app`
- **Content**: Self-contained, zero-dependency HTML5 application with embedded CSS and JavaScript.

### 2. Associated Tools
The following tools declare `_meta.ui.resourceUri = "ui://opendesk/viewport"`:

| Tool | Description |
|------|-------------|
| `opendesk_view` | Explicitly opens the interactive remote desktop viewport and peer controller. |
| `screenshot` | Captures the screen and attaches the interactive viewport resource for visual confirmation. |
| `opendesk_peers` | Lists available peers with the interactive management viewport attached. |

---

## Hosting OpenDesk MCP Apps in Custom Harnesses (e.g., `isanagent`)

If you are building or extending a custom agent framework with a web frontend, you can host OpenDesk MCP Apps with a lightweight postMessage bridge.

### 1. Iframe Element
Render the resource in a sandboxed iframe:

```html
<iframe
  id="mcp-app-frame"
  sandbox="allow-scripts allow-forms allow-same-origin"
  srcdoc="<!-- HTML content returned from resources/read -->"
  style="width: 100%; min-height: 480px; border: 1px solid #334155; border-radius: 10px;">
</iframe>
```

### 2. PostMessage Protocol Bridge (~50 lines)

In your host web page, listen for `message` events:

```javascript
window.addEventListener("message", async (event) => {
  const data = event.data;
  if (!data || data.jsonrpc !== "2.0") return;

  const iframe = document.getElementById("mcp-app-frame");

  // 1. Initial Handshake
  if (data.method === "ui/initialize") {
    iframe.contentWindow.postMessage({
      jsonrpc: "2.0",
      id: data.id,
      result: {
        protocolVersion: "2026-01-26",
        hostInfo: { name: "isanagent", version: "1.0.0" },
        hostContext: {
          theme: "dark", // or "light"
          displayMode: "inline"
        }
      }
    }, "*");
  }

  // 2. Dynamic Height Adjustment
  else if (data.method === "ui/notifications/size-changed") {
    if (data.params?.height) {
      iframe.style.height = `${data.params.height}px`;
    }
  }

  // 3. Proxy Tools/Call to OpenDesk MCP Server
  else if (data.method === "tools/call") {
    try {
      const toolResult = await mcpClient.callTool(data.params.name, data.params.arguments);
      iframe.contentWindow.postMessage({
        jsonrpc: "2.0",
        id: data.id,
        result: toolResult
      }, "*");
    } catch (err) {
      iframe.contentWindow.postMessage({
        jsonrpc: "2.0",
        id: data.id,
        error: { code: -32603, message: err.message }
      }, "*");
    }
  }

  // 4. Send Summary Messages to Chat
  else if (data.method === "ui/message") {
    if (data.params?.content?.text) {
      appendChatMessage({ role: "user", text: data.params.content.text });
    }
  }
});
```
