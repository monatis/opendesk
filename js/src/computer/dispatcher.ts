/**
 * ToolDispatcher — maps "tool.<name>" RPC calls to local JS tools.
 * Installed on the server side so paired controllers can drive this machine.
 */
import { Dispatcher } from "../protocol/peer.js";
import { ToolRegistry, createRegistry } from "../registry.js";
import { ToolContext, allowAllContext, PermissionDeniedError } from "../tools/base.js";
import type { AuditLog } from "../remote/audit.js";

function summarizeCall(toolName: string, params: Record<string, unknown>): string {
  const keys = Object.keys(params).filter((k) => k !== "image" && k !== "content").slice(0, 2);
  if (!keys.length) return toolName;
  const kv = keys.map((k) => `${k}=${String(params[k]).slice(0, 20)}`).join(" ");
  return `${toolName}(${kv})`;
}

export interface DispatcherOpts {
  registry?: ToolRegistry;
  ctx?: ToolContext;
  audit?: AuditLog;
  peerName?: string;
  peerFingerprint?: string;
  sessionId?: string;
}

export class ToolDispatcher implements Dispatcher {
  private registry: ToolRegistry;
  private ctx: ToolContext;
  private audit?: AuditLog;
  private peerName: string;
  private peerFingerprint: string;
  private sessionId: string;

  constructor(opts: DispatcherOpts = {}) {
    this.registry = opts.registry ?? createRegistry();
    this.ctx = opts.ctx ?? allowAllContext();
    this.audit = opts.audit;
    this.peerName = opts.peerName ?? "unknown";
    this.peerFingerprint = opts.peerFingerprint ?? "?";
    this.sessionId = opts.sessionId ?? "?";
  }

  async call(method: string, params: Record<string, unknown>): Promise<Record<string, unknown> | null> {
    if (!method.startsWith("tool.")) {
      throw Object.assign(new Error(`unknown method: ${method}`), { code: "not_found" });
    }
    const toolName = method.slice(5);
    let tool;
    try {
      tool = this.registry.get(toolName);
    } catch {
      throw Object.assign(new Error(`unknown tool: ${toolName}`), { code: "not_found" });
    }

    const t0 = Date.now();
    let outcome: "ok" | "error" | "denied" = "ok";
    let errorCode: string | undefined;

    try {
      const result = await tool.execute(this.ctx, params);
      if (result.error) outcome = "error";
      this.audit?.recordCall({
        peerName: this.peerName,
        peerFingerprint: this.peerFingerprint,
        sessionId: this.sessionId,
        method: toolName,
        summary: summarizeCall(toolName, params),
        outcome,
        durationMs: Date.now() - t0,
      });
      return {
        output: result.output,
        error: result.error,
        attachments: result.attachments.map((att) => ({
          mediaType: att.mediaType,
          content: att.content.toString("base64"),
          filename: att.filename,
        })),
        metadata: result.metadata,
      };
    } catch (e) {
      if (e instanceof PermissionDeniedError) {
        outcome = "denied";
        errorCode = "denied";
      } else {
        outcome = "error";
        errorCode = (e as { code?: string })?.code;
      }
      this.audit?.recordCall({
        peerName: this.peerName,
        peerFingerprint: this.peerFingerprint,
        sessionId: this.sessionId,
        method: toolName,
        summary: summarizeCall(toolName, params),
        outcome,
        errorCode,
        durationMs: Date.now() - t0,
      });
      throw e;
    }
  }

  stream(_method: string, _params: Record<string, unknown>): AsyncIterable<Record<string, unknown>> {
    return (async function* () {
      throw Object.assign(new Error("streaming not supported"), { code: "not_found" });
    })();
  }
}
