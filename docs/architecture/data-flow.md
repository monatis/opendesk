# Data Flow

## Local call

```
LLM
  │ tools/call: name + arguments
  ▼
McpServer::call_tool
  │ resolve target → "local"
  │ mouse/keyboard: check input guard (foreground window)
  ▼
LocalComputer (xa11y / SendInput / arboard / process runner)
  ▼
MCP result (text, image) → LLM
```

## Remote call

```
LLM
  │ tools/call: name + arguments (optional peer)
  ▼
McpServer::call_tool
  │ resolve target → peer name → cached or new RemoteComputer
  ▼
RemoteComputer::screenshot_format(...)   (guard attached to input.* calls)
  ▼
REQ "display.capture" → msgpack → ChaCha20-Poly1305 → binary WebSocket
  ┌────────── over the wire (direct or via rendezvous relay) ──────────┐
  ▼
WebSocket → decrypt → decode → dispatch_call on the controlled machine
  │ input.*: check guard against its own foreground window
  ▼
LocalComputer on the remote machine  (call written to the audit log)
  ▼
RES → encode → encrypt → back through the wire → RemoteComputer
  ▼
MCP result "[on PEER] ..." → LLM
```

---

Want to extend this with your own tools? See [Adding a Custom Tool →](custom-tools.md)
