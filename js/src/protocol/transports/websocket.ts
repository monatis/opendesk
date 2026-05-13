/**
 * WebSocket transport — mirrors Python opendesk.protocol.transports.websocket.
 *
 * Each opendesk protocol frame is one WebSocket binary message.
 * Uses the `ws` package.
 */

import WebSocket, { WebSocketServer as WsServer } from "ws";
import { Connection, ConnectionClosed } from "../connection.js";

interface Waiter {
  resolve: (data: Buffer) => void;
  reject: (err: Error) => void;
}

export class WebSocketConnection extends Connection {
  private ws: WebSocket;
  private queue: Buffer[] = [];
  private waiters: Waiter[] = [];
  private _closed = false;
  private _closeError?: Error;

  constructor(ws: WebSocket) {
    super();
    this.ws = ws;

    ws.on("message", (data) => {
      const buf = Buffer.isBuffer(data) ? data : Buffer.from(data as ArrayBuffer);
      if (this.waiters.length > 0) {
        this.waiters.shift()!.resolve(buf);
      } else {
        this.queue.push(buf);
      }
    });

    const onClose = () => {
      this._closed = true;
      const err = this._closeError ?? new ConnectionClosed("WebSocket closed");
      for (const w of this.waiters) w.reject(err);
      this.waiters = [];
    };

    ws.on("close", onClose);
    ws.on("error", (err) => {
      this._closeError = new ConnectionClosed(err.message);
      // close event follows error event; waiters drained there
    });
  }

  async send(data: Buffer): Promise<void> {
    if (this._closed) throw new ConnectionClosed();
    return new Promise((resolve, reject) => {
      this.ws.send(data, (err) => {
        if (err) reject(new ConnectionClosed(err.message));
        else resolve();
      });
    });
  }

  async recv(): Promise<Buffer> {
    if (this._closed && this.queue.length === 0) {
      throw this._closeError ?? new ConnectionClosed();
    }
    if (this.queue.length > 0) return this.queue.shift()!;
    return new Promise((resolve, reject) => {
      this.waiters.push({ resolve, reject });
    });
  }

  async close(): Promise<void> {
    if (this._closed) return;
    this._closed = true;
    this.ws.terminate();
    const err = new ConnectionClosed("local close");
    for (const w of this.waiters) w.reject(err);
    this.waiters = [];
  }
}

export async function connectWebSocket(url: string): Promise<WebSocketConnection> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url, { perMessageDeflate: false });
    ws.once("open", () => resolve(new WebSocketConnection(ws)));
    ws.once("error", (err) => reject(new Error(`WebSocket connect failed: ${err.message}`)));
  });
}

export type ConnectionHandler = (conn: WebSocketConnection) => Promise<void>;

export interface ServeOptions {
  host?: string;
  port?: number;
}

export class WebSocketServerWrapper {
  private server: WsServer;
  readonly port: number;
  readonly host: string;

  constructor(server: WsServer, host: string, port: number) {
    this.server = server;
    this.host = host;
    this.port = port;
  }

  async close(): Promise<void> {
    await new Promise<void>((resolve, reject) => {
      this.server.close((err) => (err ? reject(err) : resolve()));
    });
  }
}

export async function serveWebSocket(
  handler: ConnectionHandler,
  opts: ServeOptions = {},
): Promise<WebSocketServerWrapper> {
  const host = opts.host ?? "0.0.0.0";
  const port = opts.port ?? 8423;

  return new Promise((resolve, reject) => {
    const server = new WsServer({ host, port: port === 0 ? undefined : port });

    server.on("connection", async (ws) => {
      const conn = new WebSocketConnection(ws);
      try {
        await handler(conn);
      } catch {
        // ignore per-connection errors at server level
      } finally {
        await conn.close().catch(() => {});
      }
    });

    server.once("error", reject);
    server.once("listening", () => {
      const addr = server.address() as { port: number };
      resolve(new WebSocketServerWrapper(server, host, addr?.port ?? port));
    });
  });
}
