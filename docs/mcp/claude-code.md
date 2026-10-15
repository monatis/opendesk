# Claude Code

```bash
opendesk install
```

This runs `claude mcp add opendesk --scope=user -- <path to opendesk-mcp>`
(replacing any existing `opendesk` entry). Options:

- `--scope user|project|local` (default `user`)
- `--rendezvous URL` / `--rendezvous-token TOKEN` — passed through to the MCP server

The `claude` CLI must be on your PATH.

Manual equivalent:

```bash
claude mcp add opendesk -- opendesk-mcp
```

Verify it was added:

```bash
claude mcp list
```

To remove it later:

```bash
opendesk uninstall
```

---

Now start a conversation in Claude Code and ask:
> "Take a screenshot and tell me what's on my screen."

Claude will call the `screenshot` tool, receive the image, and describe it.

---

Next up: [Claude Desktop →](claude-desktop.md)
