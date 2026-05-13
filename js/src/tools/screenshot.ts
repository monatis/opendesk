import { Tool, ToolContext, ToolResult, checkPermission } from "./base.js";
import { getSandbox } from "../computer/sandbox.js";

export class ScreenshotTool extends Tool {
  name = "screenshot";
  description =
    "Capture a screenshot of the current screen or a sub-region. " +
    "Pass marks=true to overlay numbered boxes on interactive elements (Set-of-Marks).";

  schema = {
    type: "object",
    properties: {
      marks: { type: "boolean", description: "Overlay Set-of-Marks on interactive elements", default: false },
      savePath: { type: "string", description: "Absolute path to save the PNG on disk" },
      region: { type: "array", items: { type: "integer" }, description: "[x, y, width, height]" },
    },
  };

  async execute(ctx: ToolContext, params: Record<string, unknown>): Promise<ToolResult> {
    await checkPermission(ctx, "screenshot", "capture screen", "Take a screenshot");

    try {
      const screenshot = await import("screenshot-desktop");
      const { default: capture } = screenshot;

      let imgBuffer: Buffer = await capture({ format: "png" });

      const sandbox = getSandbox(ctx.sessionId);

      if (params.savePath && typeof params.savePath === "string") {
        const { writeFileSync } = await import("node:fs");
        writeFileSync(params.savePath, imgBuffer);
      }

      sandbox.lastScreenshot = imgBuffer;
      sandbox.recordAction("screenshot", params, `captured`);

      return {
        title: "Screenshot",
        output:
          "Screenshot captured. Pass image_width and image_height from metadata to the mouse tool for correct scaling.",
        error: false,
        attachments: [{ filename: "screenshot.png", mediaType: "image/png", content: imgBuffer }],
        metadata: {},
      };
    } catch (e) {
      return this.err("Screenshot error", `Failed to capture screenshot: ${e}`);
    }
  }
}
