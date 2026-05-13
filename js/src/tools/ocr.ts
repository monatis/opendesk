import { Tool, ToolContext, ToolResult, checkPermission } from "./base.js";
import { getSandbox } from "../computer/sandbox.js";

export class OCRTool extends Tool {
  name = "ocr";
  description = "Extract text from the screen or a screen region using OCR.";

  schema = {
    type: "object",
    properties: {
      region: { type: "array", items: { type: "integer" }, description: "[x, y, width, height] — omit for full screen" },
    },
  };

  async execute(ctx: ToolContext, params: Record<string, unknown>): Promise<ToolResult> {
    await checkPermission(ctx, "ocr", "extract text", "Run OCR on screen");

    try {
      const screenshot = await import("screenshot-desktop");
      const { createWorker } = await import("tesseract.js");

      let imgBuffer: Buffer = await screenshot.default({ format: "png" });

      // Crop to region if specified
      if (params.region && Array.isArray(params.region) && params.region.length === 4) {
        const { default: Jimp } = await import("jimp").catch(() => ({ default: null }));
        if (Jimp) {
          const [x, y, w, h] = params.region as number[];
          const img = await Jimp.read(imgBuffer);
          img.crop(x, y, w, h);
          imgBuffer = await img.getBufferAsync("image/png");
        }
      }

      const worker = await createWorker("eng");
      const { data: { text } } = await worker.recognize(imgBuffer);
      await worker.terminate();

      getSandbox(ctx.sessionId).recordAction("ocr", params, `${text.trim().length} chars`);
      return this.ok("OCR", text.trim() || "(no text detected)");
    } catch (e) {
      return this.err("OCR error", `${e}`);
    }
  }
}
