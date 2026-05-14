/**
 * Peer — call-id correlator on top of a Connection.
 * Mirrors Python opendesk.protocol.peer.
 *
 * Both client and server sides use a Peer.  After hello() + start(),
 * outbound calls go through call() / stream(); inbound REQ frames are
 * forwarded to the registered Dispatcher.
 */

import { Connection, ConnectionClosed } from "./connection.js";
import { encode, decode, CodecError } from "./codec.js";
import {
  PROTOCOL_VERSION,
  ErrorInfo,
  ErrorCode,
  HelloFrame,
  ReqFrame,
  ResFrame,
  CancelFrame,
  PushFrame,
  Frame,
  makeHello,
  makeRes,
  makePush,
} from "./frames.js";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

export class ProtocolError extends Error {
  code: string;
  details: Record<string, unknown>;
  constructor(code: string, message = "", details: Record<string, unknown> = {}) {
    super(message ? `${code}: ${message}` : code);
    this.name = "ProtocolError";
    this.code = code;
    this.details = details;
  }
}

function errorToException(err: ErrorInfo): Error {
  if (err.code === "cancelled") return new Error(`cancelled: ${err.message ?? ""}`);
  return new ProtocolError(err.code, err.message ?? "", (err.details as Record<string, unknown>) ?? {});
}

export function exceptionToError(exc: unknown): ErrorInfo {
  if (exc instanceof ProtocolError) {
    return { code: exc.code, message: exc.message, details: exc.details };
  }
  if (exc instanceof Error && exc.name === "PermissionDeniedError") {
    return { code: "permission_denied", message: exc.message };
  }
  if (exc instanceof Error && exc.name === "NotFoundError") {
    return { code: "not_found", message: exc.message };
  }
  const msg = exc instanceof Error ? exc.message : String(exc);
  return { code: "internal", message: msg };
}

// ---------------------------------------------------------------------------
// Dispatcher — interface the server-side handler must implement
// ---------------------------------------------------------------------------

export interface Dispatcher {
  call(method: string, params: Record<string, unknown>): Promise<Record<string, unknown> | null>;
  stream(method: string, params: Record<string, unknown>): AsyncIterable<Record<string, unknown>>;
}

// ---------------------------------------------------------------------------
// Push handler type
// ---------------------------------------------------------------------------

export type PushHandler = (topic: string, payload: Record<string, unknown>) => Promise<void> | void;

// ---------------------------------------------------------------------------
// Internal: simple async queue for streaming
// ---------------------------------------------------------------------------

const STREAM_END = Symbol("stream_end");

interface StreamItem {
  data?: Record<string, unknown>;
  error?: Error;
  end?: true;
}

interface Waiter<T> {
  resolve: (v: T) => void;
  reject: (e: Error) => void;
}

class AsyncQueue<T> {
  private items: (T | typeof STREAM_END)[] = [];
  private waiters: Waiter<T | typeof STREAM_END>[] = [];

  put(item: T | typeof STREAM_END): void {
    if (this.waiters.length > 0) {
      this.waiters.shift()!.resolve(item);
    } else {
      this.items.push(item);
    }
  }

  putError(err: Error): void {
    if (this.waiters.length > 0) {
      this.waiters.shift()!.reject(err);
    } else {
      // Store error as a special value — but we can only put T | STREAM_END.
      // Use a hack: reject all pending waiters when we drain.
      // For simplicity we'll just store the error by wrapping:
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      this.items.push({ _err: err } as any);
    }
  }

  async get(): Promise<T | typeof STREAM_END> {
    if (this.items.length > 0) {
      const item = this.items.shift()!;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      if (typeof item === "object" && item !== null && "_err" in (item as any)) {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        throw (item as any)._err;
      }
      return item;
    }
    return new Promise<T | typeof STREAM_END>((resolve, reject) => {
      this.waiters.push({ resolve, reject });
    });
  }
}

// ---------------------------------------------------------------------------
// Peer
// ---------------------------------------------------------------------------

export class Peer {
  private conn: Connection;
  private role: string;
  private dispatcher?: Dispatcher;
  private nextId = 1;

  private pending = new Map<number, { resolve: (r: Record<string, unknown> | null) => void; reject: (e: Error) => void }>();
  private streams = new Map<number, AsyncQueue<Record<string, unknown>>>();
  private inbound = new Map<number, AbortController>();

  private pushHandlers: PushHandler[] = [];
  private peerHello?: HelloFrame;
  private _closed = false;
  private runTask?: Promise<void>;

  constructor(
    conn: Connection,
    opts: { role?: string; dispatcher?: Dispatcher } = {},
  ) {
    this.conn = conn;
    this.role = opts.role ?? "client";
    this.dispatcher = opts.dispatcher;
  }

  get closed(): boolean { return this._closed; }
  get helloFrame(): HelloFrame | undefined { return this.peerHello; }

  // ------------------------------------------------------------------
  // Lifecycle
  // ------------------------------------------------------------------

  async hello(
    capabilities: Record<string, unknown> = {},
    opts: { principal?: string; auth?: Record<string, unknown>; error?: ErrorInfo } = {},
  ): Promise<HelloFrame> {
    await this.sendFrame(makeHello(this.role as "client" | "server", capabilities, opts));
    let data: Buffer;
    try {
      data = await this.conn.recv();
    } catch {
      throw new ProtocolError("protocol", "peer closed before HELLO");
    }
    const frame = decode(data);
    if (frame.type !== "hello") {
      throw new ProtocolError("protocol", `expected HELLO, got ${frame.type}`);
    }
    const hf = frame as HelloFrame;
    this.peerHello = hf;
    if (hf.error) {
      throw new ProtocolError(hf.error.code, hf.error.message ?? "", (hf.error.details as Record<string, unknown>) ?? {});
    }
    return hf;
  }

  start(): void {
    if (this.runTask || this._closed) return;
    this.runTask = this.run();
  }

  async waitClosed(): Promise<void> {
    if (this.runTask) await this.runTask.catch(() => {});
  }

  async close(reason?: Error): Promise<void> {
    if (this._closed) return;
    this._closed = true;
    this.failAll(reason ?? new ConnectionClosed("peer closed"));
    await this.conn.close().catch(() => {});
  }

  // ------------------------------------------------------------------
  // Outbound
  // ------------------------------------------------------------------

  async call(method: string, params?: Record<string, unknown>): Promise<Record<string, unknown> | null> {
    if (this._closed) throw new ConnectionClosed("peer is closed");
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.sendFrame({ v: PROTOCOL_VERSION, type: "req", id, method, params: params ?? {}, stream: false })
        .catch((err) => {
          this.pending.delete(id);
          reject(err);
        });
    });
  }

  async *stream(method: string, params?: Record<string, unknown>): AsyncIterable<Record<string, unknown>> {
    if (this._closed) throw new ConnectionClosed("peer is closed");
    const id = this.nextId++;
    const queue = new AsyncQueue<Record<string, unknown>>();
    this.streams.set(id, queue);
    try {
      await this.sendFrame({ v: PROTOCOL_VERSION, type: "req", id, method, params: params ?? {}, stream: true });
      while (true) {
        const item = await queue.get();
        if (item === STREAM_END) break;
        yield item as Record<string, unknown>;
      }
    } finally {
      this.streams.delete(id);
      if (!this._closed) {
        await this.sendFrame({ v: PROTOCOL_VERSION, type: "cancel", id, reason: "caller done" }).catch(() => {});
      }
    }
  }

  onPush(handler: PushHandler): void {
    this.pushHandlers.push(handler);
  }

  async push(topic: string, payload?: Record<string, unknown>): Promise<void> {
    await this.sendFrame(makePush(topic, payload));
  }

  // ------------------------------------------------------------------
  // Recv loop
  // ------------------------------------------------------------------

  private async run(): Promise<void> {
    try {
      while (true) {
        let data: Buffer;
        try {
          data = await this.conn.recv();
        } catch {
          break;
        }
        let frame: Frame;
        try {
          frame = decode(data);
        } catch {
          break;
        }
        await this.dispatch(frame);
      }
    } finally {
      this._closed = true;
      this.failAll(new ConnectionClosed("peer disconnected"));
    }
  }

  private async dispatch(frame: Frame): Promise<void> {
    if (frame.type === "res") return this.dispatchRes(frame as ResFrame);
    if (frame.type === "req") return this.dispatchReq(frame as ReqFrame);
    if (frame.type === "cancel") return this.dispatchCancel(frame as CancelFrame);
    if (frame.type === "push") return this.dispatchPush(frame as PushFrame);
    if (frame.type === "hello") { this.peerHello = frame as HelloFrame; return; }
  }

  private dispatchRes(frame: ResFrame): void {
    const pend = this.pending.get(frame.id);
    if (pend) {
      this.pending.delete(frame.id);
      if (frame.error) pend.reject(errorToException(frame.error));
      else pend.resolve(frame.result ?? null);
      return;
    }
    const queue = this.streams.get(frame.id);
    if (queue) {
      if (frame.error) {
        queue.putError(errorToException(frame.error));
        queue.put(STREAM_END);
        return;
      }
      if (frame.result !== null && frame.result !== undefined) {
        queue.put(frame.result);
      }
      if (frame.end) {
        queue.put(STREAM_END);
      }
    }
  }

  private async dispatchReq(frame: ReqFrame): Promise<void> {
    if (!this.dispatcher) {
      await this.sendFrame(makeRes(frame.id, {
        error: { code: "internal", message: "no dispatcher registered" },
      })).catch(() => {});
      return;
    }
    const ac = new AbortController();
    this.inbound.set(frame.id, ac);
    (frame.stream ? this.handleReqStream(frame, ac) : this.handleReqUnary(frame, ac))
      .finally(() => this.inbound.delete(frame.id));
  }

  private dispatchCancel(frame: CancelFrame): void {
    this.inbound.get(frame.id)?.abort();
  }

  private async dispatchPush(frame: PushFrame): Promise<void> {
    for (const h of this.pushHandlers) {
      try { await h(frame.topic, frame.payload ?? {}); } catch { /* ignore */ }
    }
  }

  private async handleReqUnary(frame: ReqFrame, ac: AbortController): Promise<void> {
    try {
      const result = await this.dispatcher!.call(frame.method, frame.params ?? {});
      await this.sendFrame(makeRes(frame.id, { result })).catch(() => {});
    } catch (exc) {
      await this.sendFrame(makeRes(frame.id, { error: exceptionToError(exc) })).catch(() => {});
    }
  }

  private async handleReqStream(frame: ReqFrame, ac: AbortController): Promise<void> {
    let seq = 0;
    try {
      for await (const item of this.dispatcher!.stream(frame.method, frame.params ?? {})) {
        if (ac.signal.aborted) break;
        await this.sendFrame(makeRes(frame.id, { seq, end: false, result: item })).catch(() => {});
        seq++;
      }
      await this.sendFrame(makeRes(frame.id, { seq, end: true, result: null })).catch(() => {});
    } catch (exc) {
      await this.sendFrame(makeRes(frame.id, { seq, end: true, error: exceptionToError(exc) })).catch(() => {});
    }
  }

  // ------------------------------------------------------------------
  // Helpers
  // ------------------------------------------------------------------

  private async sendFrame(frame: Frame): Promise<void> {
    await this.conn.send(encode(frame));
  }

  private failAll(err: Error): void {
    for (const { reject } of this.pending.values()) reject(err);
    this.pending.clear();
    for (const queue of this.streams.values()) {
      queue.putError(err);
      queue.put(STREAM_END);
    }
    this.streams.clear();
  }
}
