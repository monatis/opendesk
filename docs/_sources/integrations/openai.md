# OpenAI Function Calling

**Not available in the Rust version.** There is no `OpenAIAdapter`.

To use opendesk with OpenAI or an OpenAI-compatible endpoint, run an MCP
client in your agent, connect it to `opendesk-mcp` over stdio, convert the
listed tool schemas (`inputSchema`) to function definitions, and forward each
function call as an MCP `tools/call`. See [MCP Integration](../mcp/index.md).

---

Using LangChain or LangGraph? See [LangChain →](langchain.md)
