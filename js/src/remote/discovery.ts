/**
 * LAN discovery via mDNS — mirrors Python opendesk.remote.discovery.
 *
 * Service type: _opendesk._tcp.local.
 * TXT records:  pk (32-byte public key), v (version), fp (fingerprint), desc (description)
 *
 * Uses the `bonjour-service` npm package.
 */

import os from "os";
import { fingerprint } from "../protocol/auth/identity.js";

const SERVICE_TYPE = "opendesk";
const SERVICE_PROTOCOL = "tcp";

export interface DiscoveredPeer {
  name: string;
  host: string;
  port: number;
  publicKey: Buffer;
  fingerprint: string;
  description: string;
}

// ---------------------------------------------------------------------------
// Advertise
// ---------------------------------------------------------------------------

export interface Advertisement {
  close(): void;
}

export async function advertise(opts: {
  name: string;
  port: number;
  publicKey: Buffer;
  description?: string;
}): Promise<Advertisement> {
  const { Bonjour } = await import("bonjour-service");
  const bonjour = new Bonjour();

  const fp = fingerprint(opts.publicKey);
  const txt: Record<string, string> = {
    v: "1",
    pk: opts.publicKey.toString("hex"),
    fp,
  };
  if (opts.description) {
    txt["desc"] = opts.description.slice(0, 120);
  }

  const service = bonjour.publish({
    name: opts.name,
    type: `${SERVICE_TYPE}.${SERVICE_PROTOCOL}`,
    port: opts.port,
    txt,
  });

  return {
    close: () => {
      service.stop?.();
      bonjour.destroy();
    },
  };
}

// ---------------------------------------------------------------------------
// Discover
// ---------------------------------------------------------------------------

export async function discover(timeout = 2000): Promise<DiscoveredPeer[]> {
  const { Bonjour } = await import("bonjour-service");
  const bonjour = new Bonjour();
  const peers = new Map<string, DiscoveredPeer>();

  return new Promise((resolve) => {
    const browser = bonjour.find({ type: `${SERVICE_TYPE}.${SERVICE_PROTOCOL}` }, (service) => {
      try {
        const txt = (service as unknown as Record<string, unknown>)["txt"] as Record<string, string> | undefined;
        if (!txt) return;
        const pkHex = txt["pk"];
        if (!pkHex || pkHex.length !== 64) return;
        const pk = Buffer.from(pkHex, "hex");
        const fp = txt["fp"] ?? fingerprint(pk);
        const desc = txt["desc"] ?? "";

        // Prefer IPv4 address
        const addrs: string[] = (service as unknown as Record<string, unknown>)["addresses"] as string[] ?? [];
        const host = addrs.find((a) => !a.includes(":")) ?? service.host ?? "unknown";

        peers.set(pkHex, {
          name: service.name,
          host,
          port: service.port,
          publicKey: pk,
          fingerprint: fp,
          description: desc,
        });
      } catch { /* skip malformed */ }
    });

    setTimeout(() => {
      browser.stop();
      bonjour.destroy();
      resolve([...peers.values()]);
    }, timeout);
  });
}

// ---------------------------------------------------------------------------
// Local IP helper (used by server for advertising)
// ---------------------------------------------------------------------------

export function localIPv4s(): string[] {
  const addrs: string[] = [];
  for (const ifaces of Object.values(os.networkInterfaces())) {
    for (const iface of ifaces ?? []) {
      if (iface.family === "IPv4" && !iface.internal) {
        addrs.push(iface.address);
      }
    }
  }
  return addrs.length ? addrs : ["127.0.0.1"];
}
