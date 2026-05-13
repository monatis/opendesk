/**
 * Standalone MCP server — exposes all native opendesk tools over the
 * Model Context Protocol. No Python required.
 *
 * Usage:
 *   npx opendesk-js mcp          — run over stdio (for Claude Code / Claude Desktop)
 *
 * Or programmatically:
 *   import { createMcpServer, runMcpStdio } from "@vitalops/opendesk-sdk";
 *   const server = createMcpServer();
 *   // connect your own transport
 */

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { ListToolsRequestSchema, CallToolRequestSchema } from "@modelcontextprotocol/sdk/types.js";
import { createRegistry, ToolRegistry } from "./registry.js";
import { allowAllContext, ToolContext } from "./tools/base.js";

export function createMcpServer(registry?: ToolRegistry, ctx?: ToolContext): Server {
  const reg = registry ?? createRegistry();
  const context = ctx ?? allowAllContext();

  const server = new Server(
    { name: "opendesk", version: "0.1.0" },
    { capabilities: { tools: {} } },
  );

  server.setRequestHandler(ListToolsRequestSchema, async () => ({
    tools: reg.all().map((t) => ({
      name: t.name,
      description: t.description,
      inputSchema: t.schema,
    })),
  }));

  server.setRequestHandler(CallToolRequestSchema, async (request) => {
    const { name, arguments: args = {} } = request.params;
    const tool = reg.get(name);
    const result = await tool.execute(context, args as Record<string, unknown>);

    const contents: Array<{ type: string; text?: string; data?: string; mimeType?: string }> = [];

    if (result.output) {
      contents.push({ type: "text", text: result.output });
    }

    for (const att of result.attachments) {
      if (att.mediaType.startsWith("image/")) {
        contents.push({ type: "image", data: att.content.toString("base64"), mimeType: att.mediaType });
      } else {
        contents.push({ type: "text", text: `[Attachment: ${att.filename}]\ndata:${att.mediaType};base64,${att.content.toString("base64")}` });
      }
    }

    if (!contents.length) contents.push({ type: "text", text: "(no output)" });

    return { content: contents };
  });

  return server;
}

export async function runMcpStdio(registry?: ToolRegistry, ctx?: ToolContext): Promise<void> {
  const server = createMcpServer(registry, ctx);
  const transport = new StdioServerTransport();
  await server.connect(transport);
}
