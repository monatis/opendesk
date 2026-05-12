import { Tool, ToolContext, ToolResult, checkPermission } from "./base.js";
import { getSandbox } from "../computer/sandbox.js";

export class ClipboardTool extends Tool {
  name = "clipboard";
  description = "Read or write the system clipboard.";

  schema = {
    type: "object",
    required: ["action"],
    properties: {
      action: { type: "string", enum: ["read", "write"] },
      text: { type: "string", description: "Text to write — required for action=write" },
    },
  };

  async execute(ctx: ToolContext, params: Record<string, unknown>): Promise<ToolResult> {
    const { action, text } = params as { action: string; text?: string };
    await checkPermission(ctx, "clipboard", action, `Clipboard ${action}`);

    try {
      const clipboard = await import("clipboardy");
      const sandbox = getSandbox(ctx.sessionId);

      if (action === "read") {
        const content = await clipboard.default.read();
        sandbox.recordAction("clipboard_read", params, `${content.length} chars`);
        return this.ok("Clipboard read", content);
      } else if (action === "write") {
        if (!text) return this.err("Clipboard error", "text is required for action=write");
        await clipboard.default.write(text);
        sandbox.recordAction("clipboard_write", params, "ok");
        return this.ok("Clipboard write", `Written ${text.length} chars to clipboard.`);
      }
      return this.err("Clipboard error", `Unknown action: ${action}`);
    } catch (e) {
      return this.err("Clipboard error", `${e}`);
    }
  }
}
