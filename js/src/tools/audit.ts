import { Tool, ToolContext, ToolResult } from "./base.js";
import { getSandbox } from "../computer/sandbox.js";

export class AuditTool extends Tool {
  name = "audit";
  description =
    "Show the session audit log — every action taken so far. " +
    "Use format='summary' for a one-line count, or format='full' for the complete timestamped log.";

  schema = {
    type: "object",
    properties: {
      format: { type: "string", enum: ["summary", "full"], default: "full" },
      sessionId: { type: "string", description: "Session to inspect — defaults to current session" },
    },
  };

  async execute(ctx: ToolContext, params: Record<string, unknown>): Promise<ToolResult> {
    const format = (params.format as string) ?? "full";
    const sessionId = (params.sessionId as string) ?? ctx.sessionId;
    const sandbox = getSandbox(sessionId);

    if (format === "summary") {
      return this.ok("Audit summary", sandbox.summary());
    }

    const log = sandbox.exportAuditLog();
    if (!log.length) {
      return this.ok("Audit log", `No actions recorded yet for session '${sessionId}'.`);
    }

    const lines = [`Audit log — session '${sessionId}' (${log.length} actions)\n`];
    for (const entry of log) {
      const ts = new Date(entry.timestamp * 1000).toISOString().replace("T", " ").slice(0, 19);
      const errorTag = entry.error ? `  ERROR: ${entry.error}` : "";
      lines.push(`[${ts}] ${entry.action.padEnd(20)} ${JSON.stringify(entry.params)}${errorTag}`);
    }

    return this.ok("Audit log", lines.join("\n"));
  }
}
