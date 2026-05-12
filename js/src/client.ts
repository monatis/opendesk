/**
 * OpenDeskClient — typed bridge to the Python opendesk-mcp server.
 *
 * Spawns the Python MCP server as a child process and communicates with it
 * over stdio using the Model Context Protocol. All desktop automation is
 * handled by the Python layer — the JS SDK provides a fully typed interface.
 *
 * Usage:
 *
 *   import { OpenDeskClient } from "@opendesk/sdk";
 *
 *   const client = new OpenDeskClient();
 *   await client.connect();
 *
 *   const result = await client.screenshot({ marks: true });
 *   console.log(result.output);
 *
 *   await client.disconnect();
 */

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import type {
  AuditParams,
  AppParams,
  ClipboardParams,
  KeyboardParams,
  LearnParams,
  MouseParams,
  OcrParams,
  ScheduleParams,
  ScreenshotParams,
  ToolResult,
  UIParams,
} from "./types.js";

// Snake_case conversion for params — Python tool params use snake_case.
function toSnake(obj: Record<string, unknown>): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(obj)) {
    if (v === undefined) continue;
    const snake = k.replace(/([A-Z])/g, (m) => `_${m.toLowerCase()}`);
    result[snake] = v;
  }
  return result;
}

function parseResult(raw: unknown): ToolResult {
  const contents = raw as Array<{ type: string; text?: string; data?: string; mimeType?: string }>;
  const textParts: string[] = [];
  const attachments: ToolResult["attachments"] = [];

  for (const c of contents) {
    if (c.type === "text" && c.text) {
      textParts.push(c.text);
    } else if (c.type === "image" && c.data && c.mimeType) {
      attachments.push({
        filename: "screenshot.png",
        mediaType: c.mimeType,
        contentBase64: c.data,
      });
    }
  }

  const output = textParts.join("\n");
  return {
    title: "",
    output,
    error: output.toLowerCase().includes("error:"),
    attachments,
    metadata: {},
  };
}

export interface OpenDeskClientOptions {
  /**
   * Command used to start the Python MCP server.
   * Defaults to "opendesk-mcp" (assumes it is on PATH).
   */
  command?: string;
  /** Extra arguments to pass to the Python server process. */
  args?: string[];
}

export class OpenDeskClient {
  private client: Client;
  private transport: StdioClientTransport;
  private connected = false;

  constructor(options: OpenDeskClientOptions = {}) {
    const command = options.command ?? "opendesk-mcp";
    const args = options.args ?? [];

    this.transport = new StdioClientTransport({ command, args });
    this.client = new Client(
      { name: "opendesk-js", version: "0.1.0" },
      { capabilities: {} }
    );
  }

  /** Connect to the Python opendesk-mcp server. Must be called before any tool methods. */
  async connect(): Promise<void> {
    await this.client.connect(this.transport);
    this.connected = true;
  }

  /** Disconnect and stop the Python server process. */
  async disconnect(): Promise<void> {
    await this.client.close();
    this.connected = false;
  }

  private async call(name: string, params: Record<string, unknown>): Promise<ToolResult> {
    if (!this.connected) throw new Error("OpenDeskClient: call connect() first");
    const response = await this.client.callTool({ name, arguments: toSnake(params) });
    return parseResult(response.content);
  }

  // ---------------------------------------------------------------------------
  // Tools
  // ---------------------------------------------------------------------------

  /** Capture a screenshot. Pass marks:true to overlay Set-of-Marks on interactive elements. */
  screenshot(params: ScreenshotParams = {}): Promise<ToolResult> {
    return this.call("screenshot", params as Record<string, unknown>);
  }

  /** Control the mouse — click, scroll, drag, move, etc. */
  mouse(params: MouseParams): Promise<ToolResult> {
    return this.call("mouse", params as Record<string, unknown>);
  }

  /** Simulate keyboard input — type, press, hotkey, hold. */
  keyboard(params: KeyboardParams): Promise<ToolResult> {
    return this.call("keyboard", params as Record<string, unknown>);
  }

  /** Open, close, focus, or list applications. */
  app(params: AppParams): Promise<ToolResult> {
    return this.call("app", params as Record<string, unknown>);
  }

  /**
   * Interact with UI elements by accessible label — the primary interaction path.
   * Prefer this over mouse coordinates wherever possible.
   */
  ui(params: UIParams): Promise<ToolResult> {
    return this.call("ui", params as Record<string, unknown>);
  }

  /** Read or write the system clipboard. */
  clipboard(params: ClipboardParams): Promise<ToolResult> {
    return this.call("clipboard", params as Record<string, unknown>);
  }

  /** Extract text from the screen or a screen region via OCR. */
  ocr(params: OcrParams = {}): Promise<ToolResult> {
    return this.call("ocr", params as Record<string, unknown>);
  }

  /** Record a workflow once, then replay it on demand. */
  learn(params: LearnParams): Promise<ToolResult> {
    return this.call("learn", params as Record<string, unknown>);
  }

  /** Schedule a computer-use task to run at a specific time. */
  schedule(params: ScheduleParams): Promise<ToolResult> {
    return this.call("schedule", params as Record<string, unknown>);
  }

  /** Show the session audit log. */
  audit(params: AuditParams = {}): Promise<ToolResult> {
    return this.call("audit", params as Record<string, unknown>);
  }

  // ---------------------------------------------------------------------------
  // Convenience helpers
  // ---------------------------------------------------------------------------

  /** List all tools available on the connected server. */
  async listTools(): Promise<string[]> {
    if (!this.connected) throw new Error("OpenDeskClient: call connect() first");
    const { tools } = await this.client.listTools();
    return tools.map((t) => t.name);
  }
}
