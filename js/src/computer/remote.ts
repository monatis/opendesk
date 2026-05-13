/**
 * RemoteComputer — drive a paired opendesk peer over the network.
 * Mirrors Python opendesk.computer.remote.
 *
 * Every method packages its arguments into a params dict, calls
 * peer.call(method, params), and returns the decoded result.
 * Capabilities are served from the cached manifest (no round-trip).
 */

import { Peer, ProtocolError } from "../protocol/peer.js";
import { ConnectionClosed } from "../protocol/connection.js";
import { Session } from "../protocol/auth/handshake.js";

export class SessionEvicted extends Error {
  reason: string;
  constructor(reason = "") {
    super(`session evicted by server${reason ? ": " + reason : ""}`);
    this.name = "SessionEvicted";
    this.reason = reason;
  }
}

export interface CapabilityManifest {
  backend?: string;
  capabilities?: string[];
  description?: string;
  [key: string]: unknown;
}

export interface Connector {
  (): Promise<{ peer: Peer; manifest: CapabilityManifest }>;
}

const IDEMPOTENT_METHODS = new Set([
  "system.environment",
  "display.capture", "display.cursor_position", "display.displays",
  "windows.list", "windows.focused",
  "ui.tree",
  "clipboard.read",
  "fs.read", "fs.list", "fs.stat",
  "process.list",
  "apps.list",
  "notifications.list",
]);

const RECONNECT_DELAYS = [0.5, 1.0, 2.0, 4.0, 5.0, 5.0, 5.0, 5.0];

function isTransient(exc: unknown): boolean {
  if (exc instanceof ConnectionClosed) return true;
  if (exc instanceof ProtocolError && exc.code === "protocol") return true;
  return false;
}

export class RemoteComputer {
  private peer: Peer;
  private manifest: CapabilityManifest;
  private connector?: Connector;
  private reconnectBudget: number;
  private _closed = false;
  private _evicted = false;
  private _evictionReason = "";
  private reconnecting?: Promise<boolean>;

  constructor(
    peer: Peer,
    manifest: CapabilityManifest,
    opts: { connector?: Connector; reconnectBudget?: number } = {},
  ) {
    this.peer = peer;
    this.manifest = manifest;
    this.connector = opts.connector;
    this.reconnectBudget = opts.reconnectBudget ?? 30;
    this.peer.onPush(this.onPush.bind(this));
  }

  static async connect(
    session: Session,
    localCapabilities?: Record<string, unknown>,
  ): Promise<RemoteComputer> {
    const peer = new Peer(session.connection, { role: "client" });
    const hello = await peer.hello(localCapabilities ?? {});
    const manifest: CapabilityManifest = (hello.capabilities as CapabilityManifest) ?? {};
    peer.start();
    return new RemoteComputer(peer, manifest);
  }

  static async connectWithReconnect(
    connector: Connector,
    opts: { reconnectBudget?: number } = {},
  ): Promise<RemoteComputer> {
    const { peer, manifest } = await connector();
    return new RemoteComputer(peer, manifest, { connector, reconnectBudget: opts.reconnectBudget });
  }

  get evicted(): boolean { return this._evicted; }
  get evictionReason(): string { return this._evictionReason; }

  capabilities(): CapabilityManifest { return this.manifest; }

  async close(): Promise<void> {
    this._closed = true;
    await this.peer.close();
  }

  private async onPush(topic: string, payload: Record<string, unknown>): Promise<void> {
    if (topic === "session.evicted") {
      this._evicted = true;
      this._evictionReason = String(payload?.["reason"] ?? "");
    }
  }

  // ------------------------------------------------------------------
  // Core call helper with reconnect
  // ------------------------------------------------------------------

  async call(method: string, params?: Record<string, unknown>): Promise<Record<string, unknown> | null> {
    if (this._evicted) throw new SessionEvicted(this._evictionReason);
    try {
      return await this.peer.call(method, params);
    } catch (exc) {
      if (this._evicted) throw new SessionEvicted(this._evictionReason);
      if (!isTransient(exc) || !this.connector || this._closed) throw exc;
      const ok = await this.reconnect();
      if (!ok || !IDEMPOTENT_METHODS.has(method)) throw exc;
      return await this.peer.call(method, params);
    }
  }

  async *streamCall(method: string, params?: Record<string, unknown>): AsyncIterable<Record<string, unknown>> {
    if (this._evicted) throw new SessionEvicted(this._evictionReason);
    try {
      yield* this.peer.stream(method, params);
    } catch (exc) {
      throw exc;
    }
  }

  private async reconnect(): Promise<boolean> {
    if (!this.connector || this._closed || this._evicted) return false;
    // Coalesce concurrent reconnects
    if (this.reconnecting) return this.reconnecting;
    this.reconnecting = this.doReconnect().finally(() => { this.reconnecting = undefined; });
    return this.reconnecting;
  }

  private async doReconnect(): Promise<boolean> {
    await this.peer.close().catch(() => {});
    const start = Date.now();
    for (const delay of RECONNECT_DELAYS) {
      if ((Date.now() - start) / 1000 > this.reconnectBudget) break;
      try {
        const { peer, manifest } = await this.connector!();
        peer.onPush(this.onPush.bind(this));
        this.peer = peer;
        this.manifest = manifest;
        return true;
      } catch {
        if (this._closed || this._evicted) return false;
        await sleep(delay * 1000);
      }
    }
    return false;
  }

  // ------------------------------------------------------------------
  // Display
  // ------------------------------------------------------------------

  async capture(opts: { displayId?: string; region?: Record<string, unknown>; downscale?: boolean } = {}) {
    return this.call("display.capture", {
      display_id: opts.displayId ?? null,
      region: opts.region ?? null,
      downscale: opts.downscale ?? true,
    });
  }

  async cursorPosition() { return this.call("display.cursor_position"); }
  async displays() { return this.call("display.displays"); }

  subscribeDisplay(opts: { displayId?: string; fps?: number; region?: Record<string, unknown> } = {}) {
    return this.streamCall("display.subscribe", {
      display_id: opts.displayId ?? null,
      fps: opts.fps ?? 30,
      region: opts.region ?? null,
    });
  }

  // ------------------------------------------------------------------
  // Input
  // ------------------------------------------------------------------

  async pointer(event: Record<string, unknown>) { return this.call("input.pointer", { event }); }
  async key(event: Record<string, unknown>) { return this.call("input.key", { event }); }
  async typeText(text: string) { return this.call("input.text", { text_input: { text } }); }

  // ------------------------------------------------------------------
  // Windows & apps
  // ------------------------------------------------------------------

  async windows() { return this.call("windows.list"); }
  async focusedWindow() { return this.call("windows.focused"); }
  async focusWindow(windowId: string) { return this.call("windows.focus", { window_id: windowId }); }
  async openApp(name: string) { return this.call("apps.open", { name }); }
  async closeApp(name: string) { return this.call("apps.close", { name }); }
  async listApps() { return this.call("apps.list"); }

  // ------------------------------------------------------------------
  // Clipboard
  // ------------------------------------------------------------------

  async clipboardRead() { return this.call("clipboard.read"); }
  async clipboardWrite(text: string) { return this.call("clipboard.write", { contents: { text } }); }

  // ------------------------------------------------------------------
  // UI tree
  // ------------------------------------------------------------------

  async uiTree(opts: { windowId?: string; app?: string; maxDepth?: number } = {}) {
    return this.call("ui.tree", {
      window_id: opts.windowId ?? null,
      app: opts.app ?? null,
      max_depth: opts.maxDepth ?? 8,
    });
  }

  async performUiAction(element: Record<string, unknown>, action = "click", app?: string) {
    return this.call("ui.action", { element, action, app: app ?? null });
  }

  // ------------------------------------------------------------------
  // Filesystem
  // ------------------------------------------------------------------

  async readFile(path: string) { return this.call("fs.read", { path }); }
  async writeFile(path: string, data: Buffer) { return this.call("fs.write", { path, data }); }
  async listDir(path: string) { return this.call("fs.list", { path }); }
  async stat(path: string) { return this.call("fs.stat", { path }); }
  async deleteFile(path: string) { return this.call("fs.delete", { path }); }
  async move(src: string, dst: string) { return this.call("fs.move", { src, dst }); }

  // ------------------------------------------------------------------
  // Processes
  // ------------------------------------------------------------------

  async shell(cmd: string, opts: { timeout?: number; cwd?: string } = {}) {
    return this.call("process.shell", { command: cmd, timeout: opts.timeout ?? null, cwd: opts.cwd ?? null });
  }

  async processes() { return this.call("process.list"); }

  // ------------------------------------------------------------------
  // System
  // ------------------------------------------------------------------

  async environment() { return this.call("system.environment"); }
  async lockScreen() { return this.call("power.lock"); }
  async notifications() { return this.call("notifications.list"); }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
