#!/usr/bin/env node
/**
 * opendesk-mcp-bridge — MCP stdio server entry point.
 * Registered with Claude Code by `opendesk-js install`.
 */

import { runBridgeStdio } from "../dist/mcp.js";

runBridgeStdio().catch((err) => {
  console.error(err);
  process.exit(1);
});
