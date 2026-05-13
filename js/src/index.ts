/**
 * @vitalops/opendesk-sdk — public API surface
 */

export { OpenDeskClient, type OpenDeskClientOptions } from "./client.js";
export { createMcpServer, runMcpStdio } from "./mcp.js";
export { ToolRegistry, createRegistry } from "./registry.js";
export {
  Tool,
  type ToolResult,
  type ToolContext,
  type PermissionHandler,
  type Attachment,
  allowAllContext,
  checkPermission,
  PermissionDeniedError,
} from "./tools/base.js";
