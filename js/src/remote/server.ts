/**
 * opendesk serve — the controlled-machine WebSocket daemon.
 * Mirrors Python opendesk.remote.server.
 *
 * Accepts paired peers only (static-key auth).  One controller at a time:
 * a second peer trying to connect while one is active gets BUSY.
 * Optionally runs a pairing window to accept new peers.
 */

import fs from "fs";
import path from "path";
import os from "os";
import { Identity, fingerprint, generatePairingCode } from "../protocol/auth/identity.js";
import { TrustedPeers } from "../protocol/auth/storage.js";
import { authServer, pairServer, AuthFailure } from "../protocol/auth/handshake.js";
import {
  serveWebSocket,
  WebSocketServerWrapper,
  WebSocketConnection,
} from "../protocol/transports/websocket.js";
import { Peer, Dispatcher } from "../protocol/peer.js";
import type { CapabilityManifest } from "../computer/remote.js";
import { advertise, Advertisement } from "./discovery.js";

export const DEFAULT_PORT = 8423;
const DESCRIPTION_FILE = "description.txt";

export function readDescription(home?: string): string {
  const base = home ?? path.join(os.homedir(), ".opendesk");
  const f = path.join(base, DESCRIPTION_FILE);
  try {
    return fs.existsSync(f) ? fs.readFileSync(f, "utf8").trim() : "";
  } catch {
    return "";
  }
}

export function writeDescription(home: string | undefined, text: string): void {
  const base = home ?? path.join(os.homedir(), ".opendesk");
  fs.mkdirSync(base, { recursive: true, mode: 0o700 });
  fs.writeFileSync(path.join(base, DESCRIPTION_FILE), text ? text + "\n" : "", "utf8");
}

export function clearDescription(home?: string): boolean {
  const base = home ?? path.join(os.homedir(), ".opendesk");
  const f = path.join(base, DESCRIPTION_FILE);
  if (!fs.existsSync(f)) return false;
  fs.unlinkSync(f);
  return true;
}

export interface ServerSession {
  id: string;
  peerName: string;
  peerFingerprint: string;
  remoteAddr: string;
  openedAt: number;
}

export interface ServerOptions {
  host?: string;
  port?: number;
  home?: string;
  advertise?: boolean;
  dispatcher?: Dispatcher;
  /** Called to get a fresh Dispatcher for each new session. */
  dispatcherFactory?: () => Dispatcher;
}

export class OpendeskServer {
  private identity: Identity;
  private trusted: TrustedPeers;
  private opts: Required<Pick<ServerOptions, "host" | "port" | "home" | "advertise">>;
  private dispatcherFactory?: () => Dispatcher;

  private wsServer?: WebSocketServerWrapper;
  private mdnsAd?: Advertisement;
  private activeSession?: ServerSession;
  private activePeer?: Peer;

  constructor(identity: Identity, trusted: TrustedPeers, opts: ServerOptions = {}) {
    this.identity = identity;
    this.trusted = trusted;
    this.opts = {
      host: opts.host ?? "0.0.0.0",
      port: opts.port ?? DEFAULT_PORT,
      home: opts.home ?? path.join(os.homedir(), ".opendesk"),
      advertise: opts.advertise ?? true,
    };
    this.dispatcherFactory = opts.dispatcherFactory ?? (opts.dispatcher ? () => opts.dispatcher! : undefined);
  }

  get port(): number {
    return this.wsServer?.port ?? this.opts.port;
  }

  get sessions(): ServerSession[] {
    return this.activeSession ? [this.activeSession] : [];
  }

  async start(): Promise<void> {
    this.wsServer = await serveWebSocket(
      (conn) => this.handleConnection(conn),
      { host: this.opts.host, port: this.opts.port },
    );

    if (this.opts.advertise) {
      try {
        this.mdnsAd = await advertise({
          name: os.hostname(),
          port: this.wsServer.port,
          publicKey: this.identity.publicBytes,
          description: readDescription(this.opts.home),
        });
      } catch {
        // mDNS is optional; continue without it
      }
    }
  }

  async close(): Promise<void> {
    this.mdnsAd?.close();
    await this.wsServer?.close().catch(() => {});
    await this.activePeer?.close().catch(() => {});
  }

  /** Enable a one-shot pairing window.  Resolves with the new peer's public key on success. */
  async enablePairing(code: string, timeoutMs = 300_000): Promise<Buffer | null> {
    return new Promise((resolve) => {
      const timer = setTimeout(() => resolve(null), timeoutMs);

      // A second connection handler that tries pairing before standard auth
      const origHandle = this.handleConnection.bind(this);
      this.handleConnection = async (conn: WebSocketConnection) => {
        try {
          const session = await pairServer(conn, this.identity, code);
          this.trusted.add(session.peerPublic, { name: `peer-${session.peerPublic.toString("hex").slice(0, 6)}` });
          clearTimeout(timer);
          resolve(session.peerPublic);
          // Also complete the session so the peer is usable immediately
          await this.openSession(conn, session.peerPublic, "paired");
        } catch (e) {
          if (e instanceof AuthFailure && e.reason === "wrong_code") {
            // Let normal auth try it
            await origHandle(conn);
          }
        }
      };
    });
  }

  private async handleConnection(conn: WebSocketConnection): Promise<void> {
    let session;
    try {
      session = await authServer(conn, this.identity, this.trusted);
    } catch (e) {
      if (e instanceof AuthFailure) {
        // Close silently — peer gets a deliberately ambiguous response from authServer
      }
      await conn.close().catch(() => {});
      return;
    }
    await this.openSession(conn, session.peerPublic, session.connection.constructor.name);
  }

  private async openSession(
    rawConn: WebSocketConnection,
    peerPublic: Buffer,
    _label: string,
  ): Promise<void> {
    // Enforce single-controller: reject if someone is already connected
    if (this.activeSession) {
      // Send BUSY via a fresh Peer on the raw conn before upgrading to encrypted
      const tmpPeer = new Peer(rawConn, { role: "server" });
      await tmpPeer.hello(
        {},
        { error: { code: "busy", message: "another controller is active" } },
      ).catch(() => {});
      await tmpPeer.close().catch(() => {});
      return;
    }

    const peerEntry = this.trusted.find(peerPublic);
    const peerName = peerEntry?.name ?? fingerprint(peerPublic);
    const fp = fingerprint(peerPublic);

    const sessionId = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    const dispatcher = this.dispatcherFactory?.();

    const peer = new Peer(rawConn, { role: "server", dispatcher });
    const desc = readDescription(this.opts.home);
    const manifest: CapabilityManifest = {
      backend: "local",
      description: desc,
    };

    try {
      await peer.hello(manifest as Record<string, unknown>);
    } catch {
      await peer.close().catch(() => {});
      return;
    }

    this.activePeer = peer;
    this.activeSession = {
      id: sessionId,
      peerName,
      peerFingerprint: fp,
      remoteAddr: "unknown",
      openedAt: Date.now(),
    };

    peer.start();
    await peer.waitClosed();

    this.activeSession = undefined;
    this.activePeer = undefined;
  }

  /** Disconnect the active controller (cooperative eviction). */
  async disconnectActive(): Promise<boolean> {
    if (!this.activePeer || !this.activeSession) return false;
    await this.activePeer.push("session.evicted", { reason: "server requested disconnect" });
    await this.activePeer.close().catch(() => {});
    return true;
  }
}
