#!/usr/bin/env node
/**
 * opendesk-js CLI
 *
 * Commands:
 *   install [--scope user|project]  — register JS MCP bridge with Claude Code
 *   uninstall                        — remove JS MCP bridge from Claude Code
 *   mcp                              — run the MCP bridge server over stdio
 */

import { fileURLToPath } from "node:url";
import path from "node:path";
import { install, uninstall } from "../dist/install.js";
import { runBridgeStdio } from "../dist/mcp.js";

const binDir = path.dirname(fileURLToPath(import.meta.url));
const bridgeBin = path.join(binDir, "opendesk-mcp-bridge.js");

const [, , command, ...rest] = process.argv;

switch (command) {
  case "install": {
    const scopeArg = rest.find((a) => a.startsWith("--scope="));
    const scope = scopeArg ? scopeArg.split("=")[1] : "user";
    install(scope, bridgeBin);
    break;
  }
  case "uninstall":
    uninstall();
    break;
  case "mcp":
    runBridgeStdio().catch((err) => {
      console.error(err);
      process.exit(1);
    });
    break;
  default:
    console.log(`opendesk-js — JavaScript SDK for opendesk

Usage:
  opendesk-js install [--scope=user|project]   Register MCP bridge with Claude Code
  opendesk-js uninstall                         Remove MCP bridge from Claude Code
  opendesk-js mcp                               Run MCP bridge server over stdio
`);
}