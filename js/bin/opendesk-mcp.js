#!/usr/bin/env node
/**
 * opendesk native MCP server — run over stdio.
 * Used by Claude Code / Claude Desktop as the MCP command.
 *
 * Registered automatically by: npx opendesk-js install
 */

import { runMcpStdio } from "../dist/mcp.js";

runMcpStdio().catch((err) => {
  console.error(err);
  process.exit(1);
});
