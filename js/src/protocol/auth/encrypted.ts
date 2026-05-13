/**
 * ChaCha20-Poly1305 AEAD encrypted connection layer.
 * Mirrors Python opendesk.protocol.auth.encrypted.
 *
 * Per-direction counters are used as nonces; never transmitted on wire.
 * Uses Node.js built-in crypto (chacha20-poly1305, requires Node 18+).
 */

import crypto from "crypto";
import { Connection, ConnectionClosed } from "../connection.js";

const NONCE_BYTES = 12;
const TAG_BYTES = 16;
const KEY_BYTES = 32;

function makeNonce(counter: number): Buffer {
  const n = Buffer.alloc(NONCE_BYTES);
  // Write counter as 12-byte big-endian
  const hi = Math.floor(counter / 0x1_0000_0000);
  const lo = counter >>> 0;
  n.writeUInt32BE(0, 0);
  n.writeUInt32BE(0, 4);
  n.writeUInt32BE(hi, 4);
  n.writeUInt32BE(lo, 8);
  return n;
}

function chacha20Encrypt(key: Buffer, nonce: Buffer, plaintext: Buffer): Buffer {
  const cipher = crypto.createCipheriv("chacha20-poly1305", key, nonce, {
    authTagLength: TAG_BYTES,
  } as Parameters<typeof crypto.createCipheriv>[3]);
  const enc = Buffer.concat([cipher.update(plaintext), cipher.final()]);
  const tag = cipher.getAuthTag();
  return Buffer.concat([enc, tag]);
}

function chacha20Decrypt(key: Buffer, nonce: Buffer, ciphertext: Buffer): Buffer {
  const data = ciphertext.subarray(0, -TAG_BYTES);
  const tag = ciphertext.subarray(-TAG_BYTES);
  const decipher = crypto.createDecipheriv("chacha20-poly1305", key, nonce, {
    authTagLength: TAG_BYTES,
  } as Parameters<typeof crypto.createDecipheriv>[3]);
  decipher.setAuthTag(tag);
  try {
    return Buffer.concat([decipher.update(data), decipher.final()]);
  } catch {
    throw new ConnectionClosed("frame integrity check failed (AEAD tag mismatch)");
  }
}

export class EncryptedConnection extends Connection {
  private inner: Connection;
  private sendKey: Buffer;
  private recvKey: Buffer;
  private sendCounter = 0;
  private recvCounter = 0;
  private _closed = false;
  readonly peerPublic?: Buffer;

  constructor(
    inner: Connection,
    opts: { sendKey: Buffer; recvKey: Buffer; peerPublic?: Buffer },
  ) {
    super();
    if (opts.sendKey.length !== KEY_BYTES || opts.recvKey.length !== KEY_BYTES) {
      throw new Error("session keys must be 32 bytes");
    }
    this.inner = inner;
    this.sendKey = opts.sendKey;
    this.recvKey = opts.recvKey;
    this.peerPublic = opts.peerPublic;
  }

  async send(data: Buffer): Promise<void> {
    if (this._closed) throw new ConnectionClosed();
    const ct = chacha20Encrypt(this.sendKey, makeNonce(this.sendCounter), data);
    this.sendCounter++;
    await this.inner.send(ct);
  }

  async recv(): Promise<Buffer> {
    if (this._closed) throw new ConnectionClosed();
    const ct = await this.inner.recv();
    const pt = chacha20Decrypt(this.recvKey, makeNonce(this.recvCounter), ct);
    this.recvCounter++;
    return pt;
  }

  async close(): Promise<void> {
    if (this._closed) return;
    this._closed = true;
    await this.inner.close().catch(() => {});
  }
}
