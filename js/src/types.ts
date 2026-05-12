/**
 * Shared types mirroring the Python tool Params and ToolResult models.
 */

// ---------------------------------------------------------------------------
// ToolResult
// ---------------------------------------------------------------------------

export interface Attachment {
  filename: string;
  mediaType: string;
  /** Base-64 encoded content */
  contentBase64: string;
}

export interface ToolResult {
  title: string;
  output: string;
  error: boolean;
  attachments: Attachment[];
  metadata: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// screenshot
// ---------------------------------------------------------------------------

export interface ScreenshotParams {
  /** [x, y, width, height] — omit for full screen */
  region?: [number, number, number, number];
  /** Draw a red dot at the cursor position */
  showCursor?: boolean;
  /** Overlay numbered Set-of-Marks boxes on interactive elements */
  marks?: boolean;
  /** Crop region [x0, y0, x1, y1] for a close-up view */
  zoom?: [number, number, number, number];
  /** Absolute path to save the PNG on disk */
  savePath?: string;
}

// ---------------------------------------------------------------------------
// mouse
// ---------------------------------------------------------------------------

export type MouseAction =
  | "move"
  | "click"
  | "double_click"
  | "triple_click"
  | "right_click"
  | "middle_click"
  | "left_down"
  | "left_up"
  | "scroll"
  | "drag"
  | "cursor_position";

export type ScrollDirection = "up" | "down" | "left" | "right";

export interface MouseParams {
  action: MouseAction;
  x?: number;
  y?: number;
  endX?: number;
  endY?: number;
  direction?: ScrollDirection;
  amount?: number;
  duration?: number;
  /** Width of the screenshot these coordinates were read from */
  imageWidth?: number;
  /** Height of the screenshot these coordinates were read from */
  imageHeight?: number;
}

// ---------------------------------------------------------------------------
// keyboard
// ---------------------------------------------------------------------------

export type KeyboardAction = "type" | "press" | "hotkey" | "hold";

export interface KeyboardParams {
  action: KeyboardAction;
  /** Text to type — required for action="type" */
  text?: string;
  /** Key name — required for action="press" or "hold" */
  key?: string;
  /** Key names — required for action="hotkey", e.g. ["ctrl","c"] */
  keys?: string[];
  /** Seconds to hold the key — for action="hold" */
  holdDuration?: number;
}

// ---------------------------------------------------------------------------
// app
// ---------------------------------------------------------------------------

export type AppAction = "open" | "close" | "focus" | "list";

export interface AppParams {
  action: AppAction;
  /** Application name or path — required for open/close/focus */
  name?: string;
}

// ---------------------------------------------------------------------------
// clipboard
// ---------------------------------------------------------------------------

export type ClipboardAction = "read" | "write";

export interface ClipboardParams {
  action: ClipboardAction;
  /** Text to write — required for action="write" */
  text?: string;
}

// ---------------------------------------------------------------------------
// ocr
// ---------------------------------------------------------------------------

export interface OcrParams {
  /** [x, y, width, height] — omit for full screen */
  region?: [number, number, number, number];
}

// ---------------------------------------------------------------------------
// ui
// ---------------------------------------------------------------------------

export type UIAction =
  | "get_tree"
  | "click"
  | "click_menu"
  | "type"
  | "press_key"
  | "get_value";

export interface UIParams {
  action: UIAction;
  /** Target application name */
  app: string;
  /** Element label — for click / get_value */
  title?: string;
  /** Element role — for click / get_value */
  role?: string;
  /** Menu bar label — for click_menu */
  menu?: string;
  /** Menu item label — for click_menu */
  menuItem?: string;
  /** Text to type — for type */
  text?: string;
  /** Key name — for press_key */
  key?: string;
  /** Modifier keys — for press_key */
  modifiers?: string[];
}

// ---------------------------------------------------------------------------
// learn
// ---------------------------------------------------------------------------

export type LearnAction = "start" | "stop" | "save" | "replay" | "list";

export interface LearnParams {
  action: LearnAction;
  /** Task name — for start / save / replay */
  taskName?: string;
  /** Procedure JSON string — for save */
  procedure?: string;
}

// ---------------------------------------------------------------------------
// schedule
// ---------------------------------------------------------------------------

export type ScheduleAction = "add" | "remove" | "list" | "enable" | "disable";

export interface ScheduleParams {
  action: ScheduleAction;
  /** Schedule name — for add / remove / enable / disable */
  name?: string;
  /** Task description — for add */
  task?: string;
  /** Timing string — for add, e.g. "every day at 09:00" */
  timing?: string;
}

// ---------------------------------------------------------------------------
// audit
// ---------------------------------------------------------------------------

export type AuditFormat = "summary" | "full";

export interface AuditParams {
  format?: AuditFormat;
  /** Session ID to inspect — defaults to current session */
  sessionId?: string;
}
