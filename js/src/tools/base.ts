/**
 * Base abstractions for all opendesk JS tools — mirrors Python's tools/base.py.
 */

export interface Attachment {
  filename: string;
  mediaType: string;
  content: Buffer;
}

export interface ToolResult {
  title: string;
  output: string;
  error: boolean;
  attachments: Attachment[];
  metadata: Record<string, unknown>;
}

export type PermissionHandler = (tool: string, argument: string, description: string) => Promise<void>;

export class PermissionDeniedError extends Error {}

export interface ToolContext {
  sessionId: string;
  permissionHandler?: PermissionHandler;
}

export function allowAllContext(sessionId = "default"): ToolContext {
  return { sessionId };
}

export async function checkPermission(
  ctx: ToolContext,
  tool: string,
  argument: string,
  description: string,
): Promise<void> {
  if (ctx.permissionHandler) {
    await ctx.permissionHandler(tool, argument, description);
  }
}

export abstract class Tool {
  abstract name: string;
  abstract description: string;
  abstract schema: Record<string, unknown>;
  abstract execute(ctx: ToolContext, params: Record<string, unknown>): Promise<ToolResult>;

  ok(title: string, output: string, attachments: Attachment[] = [], metadata: Record<string, unknown> = {}): ToolResult {
    return { title, output, error: false, attachments, metadata };
  }

  err(title: string, output: string): ToolResult {
    return { title, output, error: true, attachments: [], metadata: {} };
  }
}
