/**
 * Abstract duplex byte-message channel — mirrors Python opendesk.protocol.connection.
 *
 * Concrete implementations:
 *   LoopbackConnection  — in-process pair (testing)
 *   WebSocketConnection — see transports/websocket.ts
 *   EncryptedConnection — see auth/encrypted.ts
 */

export class ConnectionClosed extends Error {
  constructor(msg = "connection closed") {
    super(msg);
    this.name = "ConnectionClosed";
  }
}

export abstract class Connection {
  /** Send a single binary message. */
  abstract send(data: Buffer): Promise<void>;
  /** Receive a single binary message.  Resolves when one arrives. */
  abstract recv(): Promise<Buffer>;
  /** Close the connection.  Idempotent. */
  abstract close(): Promise<void>;
}

// ---------------------------------------------------------------------------
// LoopbackConnection — two in-process halves
// ---------------------------------------------------------------------------

interface Waiter {
  resolve: (data: Buffer) => void;
  reject: (err: Error) => void;
}

class LoopbackHalf extends Connection {
  private queue: Buffer[] = [];
  private waiters: Waiter[] = [];
  private closed = false;
  private other!: LoopbackHalf;

  _link(other: LoopbackHalf) {
    this.other = other;
  }

  async send(data: Buffer): Promise<void> {
    if (this.closed) throw new ConnectionClosed();
    this.other._deliver(data);
  }

  _deliver(data: Buffer) {
    if (this.waiters.length > 0) {
      this.waiters.shift()!.resolve(data);
    } else {
      this.queue.push(data);
    }
  }

  async recv(): Promise<Buffer> {
    if (this.closed && this.queue.length === 0) throw new ConnectionClosed();
    if (this.queue.length > 0) return this.queue.shift()!;
    return new Promise((resolve, reject) => {
      this.waiters.push({ resolve, reject });
    });
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    const err = new ConnectionClosed("peer closed connection");
    for (const w of this.waiters) w.reject(err);
    this.waiters = [];
  }
}

export function loopbackPair(): [Connection, Connection] {
  const a = new LoopbackHalf();
  const b = new LoopbackHalf();
  a._link(b);
  b._link(a);
  return [a, b];
}
