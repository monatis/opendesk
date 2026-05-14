/**
 * Server-side audit log — mirrors Python opendesk.remote.audit.
 *
 * Appends one JSON line per event to ~/.opendesk/audit/<YYYY-MM-DD>.jsonl
 * File permissions: 0600. Directory: 0700.
 *
 * Event types: session.opened, session.closed, session.rejected, call
 */

import fs from "fs";
import os from "os";
import path from "path";
import { fingerprint } from "../protocol/auth/identity.js";

export interface AuditEntry {
  ts: number;               // Unix seconds (float)
  type: string;
  peer?: { name: string; fp: string };
  session_id?: string;
  remote_addr?: string;
  mode?: string;            // "serve" | "pair"
  method?: string;
  summary?: string;
  outcome?: "ok" | "error" | "denied";
  error_code?: string;
  duration_ms?: number;     // call duration
  duration?: number;        // session duration in seconds
  reason?: string;
}

export class AuditLog {
  readonly directory: string;

  constructor(opts: { home?: string } = {}) {
    const base = opts.home ?? path.join(os.homedir(), ".opendesk");
    this.directory = path.join(base, "audit");
  }

  private todayIso(): string {
    return new Date().toISOString().slice(0, 10);
  }

  filePath(date?: string): string {
    return path.join(this.directory, `${date ?? this.todayIso()}.jsonl`);
  }

  private write(entry: AuditEntry): void {
    try {
      fs.mkdirSync(this.directory, { recursive: true, mode: 0o700 });
      fs.appendFileSync(
        this.filePath(),
        JSON.stringify(entry) + "\n",
        { encoding: "utf8", mode: 0o600 },
      );
    } catch { /* audit failures must never crash the server */ }
  }

  recordSessionOpened(opts: {
    peerPublic: Buffer;
    peerName: string;
    sessionId: string;
    remoteAddr: string;
    mode: string;
  }): void {
    this.write({
      ts: Date.now() / 1000,
      type: "session.opened",
      peer: { name: opts.peerName, fp: opts.peerPublic.length ? fingerprint(opts.peerPublic) : "?" },
      session_id: opts.sessionId,
      remote_addr: opts.remoteAddr,
      mode: opts.mode,
    });
  }

  recordSessionClosed(opts: {
    peerPublic: Buffer;
    peerName: string;
    sessionId: string;
    duration: number;
    reason: string;
  }): void {
    this.write({
      ts: Date.now() / 1000,
      type: "session.closed",
      peer: { name: opts.peerName, fp: opts.peerPublic.length ? fingerprint(opts.peerPublic) : "?" },
      session_id: opts.sessionId,
      duration: opts.duration,
      reason: opts.reason,
    });
  }

  recordSessionRejected(opts: {
    peerPublic: Buffer;
    peerName: string;
    remoteAddr: string;
    reason: string;
  }): void {
    this.write({
      ts: Date.now() / 1000,
      type: "session.rejected",
      peer: { name: opts.peerName || "?", fp: opts.peerPublic.length ? fingerprint(opts.peerPublic) : "?" },
      remote_addr: opts.remoteAddr,
      reason: opts.reason,
    });
  }

  recordCall(opts: {
    peerName: string;
    peerFingerprint: string;
    sessionId: string;
    method: string;
    summary: string;
    outcome: "ok" | "error" | "denied";
    errorCode?: string;
    durationMs: number;
  }): void {
    this.write({
      ts: Date.now() / 1000,
      type: "call",
      peer: { name: opts.peerName, fp: opts.peerFingerprint },
      session_id: opts.sessionId,
      method: opts.method,
      summary: opts.summary,
      outcome: opts.outcome,
      error_code: opts.errorCode,
      duration_ms: opts.durationMs,
    });
  }

  iterEntries(date?: string): AuditEntry[] {
    const fp = this.filePath(date);
    if (!fs.existsSync(fp)) return [];
    try {
      return fs.readFileSync(fp, "utf8")
        .split("\n")
        .filter(Boolean)
        .map((line) => JSON.parse(line) as AuditEntry);
    } catch { return []; }
  }
}
