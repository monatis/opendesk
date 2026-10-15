# Anthropic SDK

**Not available in the Rust version.** There is no Python package and no
`ClaudeCodeAdapter` / `create_registry` API.

To drive opendesk from your own Claude-based agent, connect to the
`opendesk-mcp` server over MCP (stdio) and pass the tools it lists to the
model. Any MCP client library works; see [MCP Integration](../mcp/index.md).

For Claude Code itself, run `opendesk install` (see
[Claude Code](../mcp/claude-code.md)).

---

Next: [OpenAI →](openai.md)
