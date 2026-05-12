import { Tool, ToolContext, ToolResult, checkPermission } from "./base.js";
import { getSandbox } from "../computer/sandbox.js";

export class KeyboardTool extends Tool {
  name = "keyboard";
  description = "Simulate keyboard input — type text, press a key, send a hotkey, or hold a key.";

  schema = {
    type: "object",
    required: ["action"],
    properties: {
      action: { type: "string", enum: ["type", "press", "hotkey", "hold"] },
      text: { type: "string", description: "Text to type — for action=type" },
      key: { type: "string", description: "Key name — for action=press or hold" },
      keys: { type: "array", items: { type: "string" }, description: "Key combination — for action=hotkey" },
      holdDuration: { type: "number", default: 1, description: "Seconds to hold — for action=hold" },
    },
  };

  async execute(ctx: ToolContext, params: Record<string, unknown>): Promise<ToolResult> {
    const { action, text, key, keys, holdDuration = 1 } = params as {
      action: string; text?: string; key?: string; keys?: string[]; holdDuration?: number;
    };

    await checkPermission(ctx, "keyboard", action, `Keyboard ${action}`);

    try {
      const { keyboard, Key } = await import("@nut-tree-fork/nut-js");

      const resolveKey = (k: string): unknown => {
        const upper = k.charAt(0).toUpperCase() + k.slice(1).toLowerCase();
        return (Key as Record<string, unknown>)[upper] ?? (Key as Record<string, unknown>)[k.toUpperCase()] ?? k;
      };

      switch (action) {
        case "type":
          if (!text) return this.err("Keyboard error", "text is required for action=type");
          await keyboard.type(text);
          break;
        case "press":
          if (!key) return this.err("Keyboard error", "key is required for action=press");
          await keyboard.pressKey(resolveKey(key) as never);
          await keyboard.releaseKey(resolveKey(key) as never);
          break;
        case "hotkey":
          if (!keys?.length) return this.err("Keyboard error", "keys is required for action=hotkey");
          await keyboard.pressKey(...(keys.map(resolveKey) as never[]));
          await keyboard.releaseKey(...(keys.map(resolveKey) as never[]));
          break;
        case "hold":
          if (!key) return this.err("Keyboard error", "key is required for action=hold");
          await keyboard.pressKey(resolveKey(key) as never);
          await new Promise((r) => setTimeout(r, (holdDuration as number) * 1000));
          await keyboard.releaseKey(resolveKey(key) as never);
          break;
        default:
          return this.err("Keyboard error", `Unknown action: ${action}`);
      }

      getSandbox(ctx.sessionId).recordAction(`keyboard_${action}`, params, "ok");
      return this.ok(`Keyboard ${action}`, `${action} done.`);
    } catch (e) {
      return this.err("Keyboard error", `${e}`);
    }
  }
}
