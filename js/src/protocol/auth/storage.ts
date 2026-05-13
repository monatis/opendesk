/**
 * On-disk store of trusted peers — mirrors Python opendesk.protocol.auth.storage.
 *
 * Single JSON file at <home>/trusted-peers.json.
 */

import fs from "fs";
import os from "os";
import path from "path";
import { fingerprint } from "./identity.js";

export const DEFAULT_HOME = path.join(os.homedir(), ".opendesk");
const TRUSTED_PEERS_FILE = "trusted-peers.json";
const DEFAULT_PEER_FILE = "default-peer";

export interface TrustedPeer {
  publicKey: string;        // hex
  name: string;
  pairedAt: number;         // unix timestamp
  description: string;
  descriptionOverride: string;
  lastHost: string;
  lastPort: number;
}

function fingerprintOf(peer: TrustedPeer): string {
  return fingerprint(Buffer.from(peer.publicKey, "hex"));
}

export function effectiveDescription(peer: TrustedPeer): string {
  return peer.descriptionOverride || peer.description;
}

export class TrustedPeers {
  private home: string;
  private filePath: string;

  constructor(home?: string) {
    this.home = home ?? DEFAULT_HOME;
    this.filePath = path.join(this.home, TRUSTED_PEERS_FILE);
  }

  private load(): TrustedPeer[] {
    if (!fs.existsSync(this.filePath)) return [];
    try {
      const raw = JSON.parse(fs.readFileSync(this.filePath, "utf8"));
      if (!Array.isArray(raw)) return [];
      return raw
        .filter((item: unknown) => typeof item === "object" && item !== null)
        .map((item: Record<string, unknown>) => ({
          publicKey: String(item["public_key"] ?? item["publicKey"] ?? ""),
          name: String(item["name"] ?? ""),
          pairedAt: Number(item["paired_at"] ?? item["pairedAt"] ?? 0),
          description: String(item["description"] ?? ""),
          descriptionOverride: String(item["description_override"] ?? item["descriptionOverride"] ?? ""),
          lastHost: String(item["last_host"] ?? item["lastHost"] ?? ""),
          lastPort: Number(item["last_port"] ?? item["lastPort"] ?? 0),
        }))
        .filter((p: TrustedPeer) => p.publicKey.length === 64);
    } catch {
      return [];
    }
  }

  private save(peers: TrustedPeer[]): void {
    fs.mkdirSync(this.home, { recursive: true, mode: 0o700 });
    // Use snake_case for JSON so Python can read the same file.
    const json = peers.map((p) => ({
      public_key: p.publicKey,
      name: p.name,
      paired_at: p.pairedAt,
      description: p.description,
      description_override: p.descriptionOverride,
      last_host: p.lastHost,
      last_port: p.lastPort,
    }));
    fs.writeFileSync(this.filePath, JSON.stringify(json, null, 2), "utf8");
  }

  list(): TrustedPeer[] {
    return this.load();
  }

  contains(publicKey: Buffer): boolean {
    const hex = publicKey.toString("hex");
    return this.load().some((p) => p.publicKey === hex);
  }

  find(publicKey: Buffer): TrustedPeer | undefined {
    const hex = publicKey.toString("hex");
    return this.load().find((p) => p.publicKey === hex);
  }

  findByName(name: string): TrustedPeer | undefined {
    return this.load().find((p) => p.name === name);
  }

  add(publicKey: Buffer, opts: { name?: string } = {}): TrustedPeer {
    const peers = this.load();
    const hex = publicKey.toString("hex");
    const existing = peers.findIndex((p) => p.publicKey === hex);
    if (existing >= 0) {
      const p = peers[existing];
      if (opts.name && p.name !== opts.name) {
        peers[existing] = { ...p, name: opts.name };
        this.save(peers);
      }
      return peers[existing];
    }
    const peer: TrustedPeer = {
      publicKey: hex,
      name: opts.name ?? "",
      pairedAt: Date.now() / 1000,
      description: "",
      descriptionOverride: "",
      lastHost: "",
      lastPort: 0,
    };
    peers.push(peer);
    this.save(peers);
    return peer;
  }

  remove(nameOrKey: string): boolean {
    const peers = this.load();
    const before = peers.length;
    const filtered = peers.filter((p) => p.publicKey !== nameOrKey && p.name !== nameOrKey);
    if (filtered.length === before) return false;
    this.save(filtered);
    // Clear default if it pointed at this peer.
    const defPath = this.defaultFile();
    if (fs.existsSync(defPath)) {
      try {
        const def = fs.readFileSync(defPath, "utf8").trim();
        const removedNames = new Set(
          peers.filter((p) => p.publicKey === nameOrKey || p.name === nameOrKey).map((p) => p.name),
        );
        if (removedNames.has(def)) fs.unlinkSync(defPath);
      } catch { /* ignore */ }
    }
    return true;
  }

  rename(nameOrKey: string, newName: string): boolean {
    const peers = this.load();
    const idx = peers.findIndex((p) => p.publicKey === nameOrKey || p.name === nameOrKey);
    if (idx < 0) return false;
    peers[idx] = { ...peers[idx], name: newName };
    this.save(peers);
    return true;
  }

  cacheDescription(publicKey: Buffer, description: string): boolean {
    const peers = this.load();
    const hex = publicKey.toString("hex");
    const idx = peers.findIndex((p) => p.publicKey === hex);
    if (idx < 0 || peers[idx].description === description) return false;
    peers[idx] = { ...peers[idx], description };
    this.save(peers);
    return true;
  }

  cacheEndpoint(publicKey: Buffer, host: string, port: number): boolean {
    const peers = this.load();
    const hex = publicKey.toString("hex");
    const idx = peers.findIndex((p) => p.publicKey === hex);
    if (idx < 0) return false;
    const p = peers[idx];
    if (p.lastHost === host && p.lastPort === port) return false;
    peers[idx] = { ...p, lastHost: host, lastPort: port };
    this.save(peers);
    return true;
  }

  // Persistent default peer
  private defaultFile(): string {
    return path.join(this.home, DEFAULT_PEER_FILE);
  }

  getDefault(): string | undefined {
    const f = this.defaultFile();
    if (!fs.existsSync(f)) return undefined;
    try {
      const name = fs.readFileSync(f, "utf8").trim();
      if (!name) return undefined;
      if (!this.findByName(name)) return undefined;
      return name;
    } catch {
      return undefined;
    }
  }

  setDefault(name: string): boolean {
    if (!this.findByName(name)) return false;
    fs.mkdirSync(this.home, { recursive: true, mode: 0o700 });
    fs.writeFileSync(this.defaultFile(), name + "\n", "utf8");
    return true;
  }

  clearDefault(): boolean {
    const f = this.defaultFile();
    if (!fs.existsSync(f)) return false;
    fs.unlinkSync(f);
    return true;
  }
}
