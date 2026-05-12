import { Tool, ToolContext, ToolResult, checkPermission } from "./base.js";
import { getSandbox } from "../computer/sandbox.js";

export class MouseTool extends Tool {
  name = "mouse";
  description =
    "Control the mouse — click, double-click, right-click, scroll, drag, move. " +
    "Always prefer the ui tool first. Use mouse only for elements with no accessible label.";

  schema = {
    type: "object",
    required: ["action"],
    properties: {
      action: {
        type: "string",
        enum: ["move", "click", "double_click", "right_click", "scroll", "drag", "cursor_position"],
      },
      x: { type: "integer" },
      y: { type: "integer" },
      endX: { type: "integer" },
      endY: { type: "integer" },
      direction: { type: "string", enum: ["up", "down", "left", "right"] },
      amount: { type: "integer", default: 3 },
    },
  };

  async execute(ctx: ToolContext, params: Record<string, unknown>): Promise<ToolResult> {
    const { action, x, y, endX, endY, direction, amount = 3 } = params as {
      action: string; x?: number; y?: number; endX?: number; endY?: number;
      direction?: string; amount?: number;
    };

    await checkPermission(ctx, "mouse", `${action} at (${x},${y})`, `Mouse ${action}`);

    const sandbox = getSandbox(ctx.sessionId);
    if (x !== undefined && y !== undefined && !sandbox.isCoordinateAllowed(x, y)) {
      return this.err("Mouse denied", "Coordinates outside permitted screen region.");
    }

    try {
      const { mouse, straightTo, Point, Button } = await import("@nut-tree-fork/nut-js");

      switch (action) {
        case "move":
          await mouse.move(straightTo(new Point(x!, y!)));
          break;
        case "click":
          await mouse.move(straightTo(new Point(x!, y!)));
          await mouse.click(Button.LEFT);
          break;
        case "double_click":
          await mouse.move(straightTo(new Point(x!, y!)));
          await mouse.doubleClick(Button.LEFT);
          break;
        case "right_click":
          await mouse.move(straightTo(new Point(x!, y!)));
          await mouse.click(Button.RIGHT);
          break;
        case "scroll":
          if (direction === "down") await mouse.scrollDown(amount as number);
          else if (direction === "up") await mouse.scrollUp(amount as number);
          else if (direction === "left") await mouse.scrollLeft(amount as number);
          else await mouse.scrollRight(amount as number);
          break;
        case "drag":
          await mouse.drag(straightTo(new Point(endX!, endY!)));
          break;
        case "cursor_position": {
          const pos = await mouse.getPosition();
          sandbox.recordAction("cursor_position", params, `(${pos.x},${pos.y})`);
          return this.ok("Cursor position", `x=${pos.x}, y=${pos.y}`);
        }
        default:
          return this.err("Mouse error", `Unknown action: ${action}`);
      }

      sandbox.recordAction(`mouse_${action}`, params, "ok");
      return this.ok(`Mouse ${action}`, `${action} at (${x ?? ""},${y ?? ""}) done.`);
    } catch (e) {
      return this.err("Mouse error", `${e}`);
    }
  }
}
