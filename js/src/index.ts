/**
 * @opendesk/sdk — JavaScript/TypeScript SDK for opendesk.
 *
 * Quick start:
 *
 *   import { OpenDeskClient } from "@opendesk/sdk";
 *
 *   const client = new OpenDeskClient();
 *   await client.connect();
 *
 *   // Take a screenshot with Set-of-Marks
 *   const result = await client.screenshot({ marks: true });
 *   console.log(result.output);
 *
 *   // Click a UI element by name (no coordinates needed)
 *   await client.ui({ action: "click", app: "Safari", title: "Go" });
 *
 *   await client.disconnect();
 */

export { OpenDeskClient } from "./client.js";
export type { OpenDeskClientOptions } from "./client.js";

export { createMcpBridge, runBridgeStdio } from "./mcp.js";
export type { BridgeOptions } from "./mcp.js";

export { install, uninstall } from "./install.js";

export type {
  ToolResult,
  Attachment,
  ScreenshotParams,
  MouseParams,
  MouseAction,
  ScrollDirection,
  KeyboardParams,
  KeyboardAction,
  AppParams,
  AppAction,
  ClipboardParams,
  ClipboardAction,
  OcrParams,
  UIParams,
  UIAction,
  LearnParams,
  LearnAction,
  ScheduleParams,
  ScheduleAction,
  AuditParams,
  AuditFormat,
} from "./types.js";
