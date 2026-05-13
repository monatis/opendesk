/**
 * Wire-protocol frame schemas — mirrors Python opendesk.protocol.frames.
 *
 * Five frame types cross the protocol boundary; all are msgpack-encoded
 * binary messages (one per WebSocket message).
 */

export const PROTOCOL_VERSION = 1;

export type ErrorCode =
  | "capability_unsupported"
  | "permission_denied"
  | "invalid_argument"
  | "not_found"
  | "timeout"
  | "cancelled"
  | "internal"
  | "protocol"
  | "busy";

export interface ErrorInfo {
  code: string;
  message?: string;
  details?: Record<string, unknown>;
}

export interface HelloFrame {
  v: number;
  type: "hello";
  role: "client" | "server";
  principal?: string;
  auth?: Record<string, unknown>;
  capabilities?: Record<string, unknown>;
  error?: ErrorInfo | null;
}

export interface ReqFrame {
  v: number;
  type: "req";
  id: number;
  method: string;
  stream?: boolean;
  params?: Record<string, unknown>;
}

export interface ResFrame {
  v: number;
  type: "res";
  id: number;
  seq?: number;
  end?: boolean;
  result?: Record<string, unknown> | null;
  error?: ErrorInfo | null;
}

export interface CancelFrame {
  v: number;
  type: "cancel";
  id: number;
  reason?: string;
}

export interface PushFrame {
  v: number;
  type: "push";
  topic: string;
  payload?: Record<string, unknown>;
}

export type Frame = HelloFrame | ReqFrame | ResFrame | CancelFrame | PushFrame;

export function makeHello(
  role: "client" | "server",
  capabilities: Record<string, unknown> = {},
  opts: { principal?: string; auth?: Record<string, unknown>; error?: ErrorInfo } = {},
): HelloFrame {
  return {
    v: PROTOCOL_VERSION,
    type: "hello",
    role,
    principal: opts.principal ?? "",
    auth: opts.auth ?? {},
    capabilities,
    error: opts.error ?? null,
  };
}

export function makeReq(
  id: number,
  method: string,
  params: Record<string, unknown> = {},
  stream = false,
): ReqFrame {
  return { v: PROTOCOL_VERSION, type: "req", id, method, params, stream };
}

export function makeRes(
  id: number,
  opts: { seq?: number; end?: boolean; result?: Record<string, unknown> | null; error?: ErrorInfo } = {},
): ResFrame {
  return {
    v: PROTOCOL_VERSION,
    type: "res",
    id,
    seq: opts.seq ?? 0,
    end: opts.end ?? true,
    result: opts.result ?? null,
    error: opts.error ?? null,
  };
}

export function makePush(topic: string, payload: Record<string, unknown> = {}): PushFrame {
  return { v: PROTOCOL_VERSION, type: "push", topic, payload };
}
