/**
 * Local IPC for inspecting / killing active opendesk-serve sessions.
 *
 * Unix  : domain socket at <home>/admin.sock (mode 0600 — owner only).
 * Windows: TCP on 127.0.0.1, ephemeral port stored in <home>/admin.port.
 *
 * Wire format: 4-byte big-endian payload length + UTF-8 JSON.
 * One request / response round trip per connection; server closes after reply.
 *
 * Ops:
 *   { op: "list" }               → { ok: true, sessions: [...] }
 *   { op: "kill", id: "abc123" } → { ok: true }  |  { ok: false, error: "..." }
 *   { op: "kill_all" }           → { ok: true, killed: N }
 */

import net from "net";
import fs from "fs";
import os from "os";
import path from "path";

const SOCKET_NAME = "admin.sock";
const PORT_FILE = "admin.port";
const IS_WIN = process.platform === "win32";
const MAX_FRAME = 1 * 1024 * 1024; // 1 MB sanity cap

function socketPath(home?: string): string {
  return path.join(home ?? path.join(os.homedir(), ".opendesk"), SOCKET_NAME);
}

function portFilePath(home?: string): string {
  return path.join(home ?? path.join(os.homedir(), ".opendesk"), PORT_FILE);
}

function encodeFrame(obj: unknown): Buffer {
  const payload = Buffer.from(JSON.stringify(obj), "utf8");
  const frame = Buffer.allocUnsafe(4 + payload.length);
  frame.writeUInt32BE(payload.length, 0);
  payload.copy(frame, 4);
  return frame;
}

function readFrame(sock: net.Socket): Promise<Record<string, unknown>> {
  return new Promise((resolve, reject) => {
    let buf = Buffer.alloc(0);

    const cleanup = () => {
      sock.off("data", onData);
      sock.off("error", onErr);
      sock.off("close", onClose);
    };
    const onData = (chunk: Buffer) => {
      buf = Buffer.concat([buf, chunk]);
      if (buf.length < 4) return;
      const len = buf.readUInt32BE(0);
      if (len > MAX_FRAME) { cleanup(); reject(new Error("admin frame too large")); return; }
      if (buf.length >= 4 + len) {
        cleanup();
        try { resolve(JSON.parse(buf.subarray(4, 4 + len).toString("utf8"))); }
        catch (e) { reject(new Error(`admin frame parse error: ${e}`)); }
      }
    };
    const onErr   = (e: Error) => { cleanup(); reject(e); };
    const onClose = () => { cleanup(); reject(new Error("admin connection closed")); };

    sock.on("data", onData);
    sock.on("error", onErr);
    sock.on("close", onClose);
  });
}

// Restrict session ids to safe characters to prevent protocol injection.
const SAFE_ID = /^[\w-]{1,64}$/;

// ---------------------------------------------------------------------------
// Public error type
// ---------------------------------------------------------------------------

export class AdminError extends Error {
  constructor(msg: string) { super(msg); this.name = "AdminError"; }
}

// ---------------------------------------------------------------------------
// Session shape returned by listSessions()
// ---------------------------------------------------------------------------

export interface SessionInfo {
  id: string;
  peer_name: string;
  peer_fingerprint: string;
  remote_addr: string;
  opened_at: number;   // Unix ms
  age_seconds: number;
}

// ---------------------------------------------------------------------------
// AdminClient — used by CLI commands
// ---------------------------------------------------------------------------

export class AdminClient {
  private socket: net.Socket;
  private constructor(socket: net.Socket) { this.socket = socket; }

  static async connect(home?: string): Promise<AdminClient> {
    return new AdminClient(await AdminClient.openSocket(home));
  }

  private static openSocket(home?: string): Promise<net.Socket> {
    return new Promise((resolve, reject) => {
      let sock: net.Socket;
      if (!IS_WIN) {
        const sp = socketPath(home);
        if (!fs.existsSync(sp)) {
          return reject(new AdminError("opendesk serve is not running (admin socket not found)"));
        }
        sock = net.createConnection(sp);
      } else {
        const pf = portFilePath(home);
        if (!fs.existsSync(pf)) {
          return reject(new AdminError("opendesk serve is not running (admin port file not found)"));
        }
        const port = parseInt(fs.readFileSync(pf, "utf8").trim(), 10);
        if (!port || !Number.isFinite(port) || port < 1 || port > 65535) {
          return reject(new AdminError("corrupt admin port file"));
        }
        sock = net.createConnection(port, "127.0.0.1");
      }
      sock.once("connect", () => resolve(sock));
      sock.once("error", (e) => reject(new AdminError(`admin connect failed: ${e.message}`)));
    });
  }

  async listSessions(): Promise<SessionInfo[]> {
    this.socket.write(encodeFrame({ op: "list" }));
    const resp = await readFrame(this.socket);
    if (!resp["ok"]) throw new AdminError(String(resp["error"] ?? "unknown error"));
    return (resp["sessions"] as SessionInfo[]) ?? [];
  }

  async killAll(): Promise<number> {
    this.socket.write(encodeFrame({ op: "kill_all" }));
    const resp = await readFrame(this.socket);
    if (!resp["ok"]) throw new AdminError(String(resp["error"] ?? "unknown error"));
    return Number(resp["killed"] ?? 0);
  }

  async kill(id: string): Promise<boolean> {
    if (!SAFE_ID.test(id)) throw new AdminError("invalid session id");
    this.socket.write(encodeFrame({ op: "kill", id }));
    const resp = await readFrame(this.socket);
    return Boolean(resp["ok"]);
  }

  close(): void { this.socket.destroy(); }
}

// ---------------------------------------------------------------------------
// AdminServer — started inside OpendeskServer.start()
// ---------------------------------------------------------------------------

export type SessionsProvider = () => Array<{
  id: string;
  peerName: string;
  peerFingerprint: string;
  remoteAddr: string;
  openedAt: number; // ms
}>;

export class AdminServer {
  private netServer?: net.Server;
  private home?: string;
  private getSessions: SessionsProvider;
  private killSession:    (id: string) => Promise<boolean>;
  private killAllSessions: () => Promise<number>;

  constructor(opts: {
    home?: string;
    getSessions: SessionsProvider;
    killSession:    (id: string) => Promise<boolean>;
    killAllSessions: () => Promise<number>;
  }) {
    this.home = opts.home;
    this.getSessions = opts.getSessions;
    this.killSession = opts.killSession;
    this.killAllSessions = opts.killAllSessions;
  }

  async start(): Promise<void> {
    const base = this.home ?? path.join(os.homedir(), ".opendesk");
    fs.mkdirSync(base, { recursive: true, mode: 0o700 });

    this.netServer = net.createServer((sock) => void this.handleClient(sock));

    if (!IS_WIN) {
      const sp = socketPath(this.home);
      try { fs.unlinkSync(sp); } catch { /* stale socket */ }
      await bind(this.netServer, sp);
      try { fs.chmodSync(sp, 0o600); } catch { /* best effort */ }
    } else {
      await bind(this.netServer, 0, "127.0.0.1");
      const addr = this.netServer.address() as net.AddressInfo;
      // Write port file with owner-only permissions (best-effort on Windows)
      fs.writeFileSync(portFilePath(this.home), String(addr.port) + "\n", { mode: 0o600, flag: "w" });
    }
  }

  close(): void {
    this.netServer?.close();
    try {
      fs.unlinkSync(IS_WIN ? portFilePath(this.home) : socketPath(this.home));
    } catch { /* already gone */ }
  }

  private async handleClient(sock: net.Socket): Promise<void> {
    let resp: unknown = { ok: false, error: "internal error" };
    try {
      const req = await readFrame(sock);
      const op  = String(req["op"] ?? "");

      if (op === "list") {
        const now = Date.now() / 1000;
        resp = {
          ok: true,
          sessions: this.getSessions().map((s) => ({
            id:               s.id,
            peer_name:        s.peerName,
            peer_fingerprint: s.peerFingerprint,
            remote_addr:      s.remoteAddr,
            opened_at:        s.openedAt,
            age_seconds:      now - s.openedAt / 1000,
          })),
        };
      } else if (op === "kill") {
        const id = String(req["id"] ?? "");
        if (!SAFE_ID.test(id)) {
          resp = { ok: false, error: "invalid session id" };
        } else {
          const ok = await this.killSession(id);
          resp = ok ? { ok: true } : { ok: false, error: "no such session" };
        }
      } else if (op === "kill_all") {
        const killed = await this.killAllSessions();
        resp = { ok: true, killed };
      } else {
        resp = { ok: false, error: `unknown op: ${op}` };
      }
    } catch (e) {
      resp = { ok: false, error: e instanceof Error ? e.message : String(e) };
    }

    try {
      await new Promise<void>((res, rej) =>
        sock.write(encodeFrame(resp), (err) => (err ? rej(err) : res())),
      );
    } catch { /* ignore write error */ }
    sock.destroy();
  }
}

function bind(server: net.Server, portOrPath: number | string, host?: string): Promise<void> {
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    if (typeof portOrPath === "string") {
      server.listen(portOrPath, () => { server.off("error", reject); resolve(); });
    } else {
      server.listen(portOrPath, host, () => { server.off("error", reject); resolve(); });
    }
  });
}
