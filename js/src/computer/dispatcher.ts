/**
 * ToolDispatcher — maps "tool.<name>" RPC calls to local JS tools.
 * Installed on the server side so paired controllers can drive this machine.
 */
import { Dispatcher } from "../protocol/peer.js";
import { ToolRegistry, createRegistry } from "../registry.js";
import { ToolContext, allowAllContext } from "../tools/base.js";

export class ToolDispatcher implements Dispatcher {
  private registry: ToolRegistry;
  private ctx: ToolContext;

  constructor(opts: { registry?: ToolRegistry; ctx?: ToolContext } = {}) {
    this.registry = opts.registry ?? createRegistry();
    this.ctx = opts.ctx ?? allowAllContext();
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
    const result = await tool.execute(this.ctx, params);
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
  }

  stream(_method: string, _params: Record<string, unknown>): AsyncIterable<Record<string, unknown>> {
    return (async function* () {
      throw Object.assign(new Error("streaming not supported"), { code: "not_found" });
    })();
  }
}
