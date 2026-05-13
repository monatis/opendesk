/**
 * CLI install/uninstall commands — register the JS MCP bridge with Claude Code.
 *
 * Usage:
 *   npx opendesk-js install
 *   npx opendesk-js uninstall
 */

import { execFileSync, execSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

function findClaude(): string {
  try {
    const cmd = process.platform === "win32" ? "where claude" : "which claude";
    const result = execSync(cmd, { encoding: "utf-8" }).trim().split("\n")[0].replace(/\r/g, "");
    if (result) return result;
  } catch {
    // fall through
  }
  throw new Error(
    "claude command not found.\n" +
    "Install Claude Code first: https://claude.ai/code"
  );
}
function findMcpBin(): string {
  const thisFile = fileURLToPath(import.meta.url);
  const pkgRoot = path.resolve(path.dirname(thisFile), "..");
  return path.join(pkgRoot, "bin", "opendesk-mcp.js");
}

export function install(scope: "user" | "project" = "user"): void {
  const claude = findClaude();
  const mcpBin = findMcpBin();

  // Remove existing entry if any
  try {
    execFileSync(claude, ["mcp", "remove", "opendesk-js"], { stdio: "pipe" });
  } catch {
    // Not registered yet — fine
  }

  execFileSync(
    claude,
    ["mcp", "add", "opendesk-js", `--scope=${scope}`, "--", "node", mcpBin],
    { stdio: "inherit", shell: process.platform === "win32" }
  );

  console.log(`opendesk JS MCP server registered (${scope}).`);
  console.log(`  Server: ${mcpBin}`);
  console.log("Start a Claude Code conversation and say 'take a screenshot' to verify.");
}

export function uninstall(): void {
  const claude = findClaude();
  execFileSync(claude, ["mcp", "remove", "opendesk-js"], { stdio: "inherit", shell: process.platform === "win32" });
  console.log("opendesk JS MCP server removed from Claude Code.");
}