# Integrations

The Rust version of opendesk integrates with agents in two ways:

| Integration | Guide |
|---|---|
| MCP (recommended) — any MCP client or agent framework with an MCP client | [MCP Integration](../mcp/index.md) |
| Rust programs — link the `opendesk-rs` crate | [Programmatic Use](../remote/programmatic.md) |

The Python SDK adapters of the earlier Python version
(`ClaudeCodeAdapter`, `OpenAIAdapter`, `as_langchain_tools`, the tool
registry) are **not available** in the Rust version. The pages below say what
to use instead.

| Former integration | Status |
|---|---|
| [Anthropic SDK](anthropic.md) | Not available — use MCP |
| [OpenAI / compatible](openai.md) | Not available — use MCP |
| [LangChain / LangGraph](langchain.md) | Not available — use MCP |
| [Custom](custom.md) | Use MCP over stdio, or the Rust crate |
