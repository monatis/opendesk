/**
 * MCP server bridge — exposes the Python opendesk-mcp tools through a Node.js
 * MCP server. Useful when you want to serve opendesk tools from a JS process
 * (e.g. inside a Next.js API route, an Electron app, or a custom agent loop).
 *
 * The bridge spawns the Python opendesk-mcp server, discovers its tools, and
 * re-exposes them verbatim — no tool logic lives in JS.
 *
 * Usage:
 *
 *   import { createMcpBridge } from "@opendesk/sdk/mcp";
 *
 *   const bridge = await createMcpBridge();
 *   // bridge is a running MCP Server connected over stdio
 */

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { ListToolsRequestSchema, CallToolRequestSchema } from "@modelcontextprotocol/sdk/types.js";

export interface BridgeOptions {
  /** Command to start the Python MCP server. Defaults to "opendesk-mcp". */
  pythonCommand?: string;
  args?: string[];
}

/**
 * Create an MCP server that transparently proxies all tool calls to the
 * Python opendesk-mcp process.
 *
 * Call `await bridge.start()` to begin serving over stdio.
 */
export async function createMcpBridge(options: BridgeOptions = {}): Promise<Server> {
  const command = options.pythonCommand ?? "opendesk-mcp";
  const args = options.args ?? [];

  // Connect to Python server as MCP client
  const upstream = new Client(
    { name: "opendesk-js-bridge", version: "0.1.0" },
    { capabilities: {} }
  );
  const upstreamTransport = new StdioClientTransport({ command, args });
  await upstream.connect(upstreamTransport);

  // Discover tools from Python server
  const { tools } = await upstream.listTools();

  // Create JS MCP server that re-exposes those tools
  const server = new Server(
    { name: "opendesk", version: "0.1.0" },
    { capabilities: { tools: {} } }
  );

server.setRequestHandler(
    ListToolsRequestSchema,
    async () => ({ tools })
  );

  server.setRequestHandler(
    CallToolRequestSchema,
    async (request) => {
      const { name, arguments: args = {} } = request.params;
      return upstream.callTool({ name, arguments: args });
    }
  );

  return server;
}

/**
 * Run the bridge as a stdio MCP server — entry point for the
 * `opendesk-js-mcp` CLI command.
 */
export async function runBridgeStdio(options: BridgeOptions = {}): Promise<void> {
  const server = await createMcpBridge(options);
  const transport = new StdioServerTransport();
  await server.connect(transport);
}
