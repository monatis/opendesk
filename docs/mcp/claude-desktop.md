# Claude Desktop

Edit `claude_desktop_config.json`:

- **Windows**: `%APPDATA%\Claude\claude_desktop_config.json`
- **macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`

```json
{
  "mcpServers": {
    "opendesk": {
      "command": "opendesk-mcp"
    }
  }
}
```

If `opendesk-mcp` is not on the PATH Claude Desktop sees, use the full path
(by default `~/.cargo/bin/opendesk-mcp`, or `opendesk-mcp.exe` on Windows):

```json
{
  "mcpServers": {
    "opendesk": {
      "command": "C:\\Users\\you\\.cargo\\bin\\opendesk-mcp.exe"
    }
  }
}
```

Restart Claude Desktop. The tools appear in the toolbar.

---

Next up: [Cursor →](cursor.md)
