#!/usr/bin/env node
/**
 * opendesk-js CLI
 *
 * Commands:
 *   install [--scope=user|project]  — register native MCP server with Claude Code
 *   uninstall                        — remove MCP server from Claude Code
 *   mcp                              — run the native MCP server over stdio
 */

import { install, uninstall } from "../dist/install.js";
import { runMcpStdio } from "../dist/mcp.js";

const [, , command, ...rest] = process.argv;

switch (command) {
  case "install": {
    const scopeArg = rest.find((a) => a.startsWith("--scope="));
    const scope = scopeArg ? scopeArg.split("=")[1] : "user";
    install(scope);
    break;
  }
  case "uninstall":
    uninstall();
    break;
  case "mcp":
    runMcpStdio().catch((err) => {
      console.error(err);
      process.exit(1);
    });
    break;
  default:
    console.log(`opendesk-js — JavaScript SDK for opendesk

Usage:
  opendesk-js install [--scope=user|project]   Register native MCP server with Claude Code
  opendesk-js uninstall                         Remove MCP server from Claude Code
  opendesk-js mcp                               Run native MCP server over stdio
`);
}
