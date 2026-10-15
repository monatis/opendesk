# Continue (VS Code)

Edit `.continue/config.json`:

```json
{
  "mcpServers": [
    {
      "name": "opendesk",
      "command": "opendesk-mcp",
      "transport": "stdio"
    }
  ]
}
```

---

## Any other MCP client

The pattern is always the same — point the client at the `opendesk-mcp` command with stdio transport.

---

Need options beyond the defaults? See [Advanced →](advanced.md)
