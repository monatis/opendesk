/**
 * Controller-side helpers — connect to a paired opendesk peer.
 * Mirrors Python opendesk.remote.client.
 */

import { Identity } from "../protocol/auth/identity.js";
import { TrustedPeers } from "../protocol/auth/storage.js";
import { authClient, pairClient } from "../protocol/auth/handshake.js";
import { connectWebSocket } from "../protocol/transports/websocket.js";
import { Peer } from "../protocol/peer.js";
import { RemoteComputer, CapabilityManifest } from "../computer/remote.js";
import type { DiscoveredPeer } from "./discovery.js";

export type Target = string | DiscoveredPeer;

export interface ConnectOptions {
  home?: string;
  timeout?: number;
  autoReconnect?: boolean;
  reconnectBudget?: number;
}

/**
 * Open a RemoteComputer to a paired peer.
 *
 * target can be:
 *  - a peer name stored in trusted-peers
 *  - a DiscoveredPeer from discover()
 *  - a URL like "ws://host:8423#<pubkey-hex>"
 *  - undefined → uses the persistent default peer
 */
export async function connect(
  target?: Target,
  opts: ConnectOptions = {},
): Promise<RemoteComputer> {
  const { home, timeout = 5000, autoReconnect = true, reconnectBudget = 30 } = opts;

  if (target === undefined) {
    const def = new TrustedPeers(home).getDefault();
    if (!def) throw new Error("no peer specified and no default-peer set");
    target = def;
  }

  const identity = Identity.loadOrCreate(home);

  async function openSession(): Promise<{ peer: Peer; manifest: CapabilityManifest }> {
    const { host, port, pubkey } = await resolve(target!, { home, timeout });
    const ws = await connectWebSocket(`ws://${host}:${port}`);
    let session;
    try {
      session = await authClient(ws, identity, pubkey);
    } catch (e) {
      await ws.close().catch(() => {});
      throw e;
    }
    const peer = new Peer(session.connection, { role: "client" });
    let hello;
    try {
      hello = await peer.hello({});
    } catch (e) {
      await peer.close();
      throw e;
    }
    const manifest: CapabilityManifest = (hello.capabilities as CapabilityManifest) ?? {};
    peer.start();

    // Cache endpoint + description
    const store = new TrustedPeers(home);
    store.cacheEndpoint(pubkey, host, port);
    if (manifest.description) {
      store.cacheDescription(pubkey, String(manifest.description));
    }

    return { peer, manifest };
  }

  if (!autoReconnect) {
    const { peer, manifest } = await openSession();
    return new RemoteComputer(peer, manifest);
  }

  return RemoteComputer.connectWithReconnect(() => openSession(), { reconnectBudget });
}

export interface PairWithOptions {
  home?: string;
  name?: string;
}

/**
 * Pair with a peer at host:port using a 6-digit code.
 * Returns the RemoteComputer and the server's public key.
 */
export async function pairWith(
  host: string,
  port: number,
  code: string,
  opts: PairWithOptions = {},
): Promise<{ remote: RemoteComputer; serverPubkey: Buffer }> {
  const { home, name } = opts;
  const identity = Identity.loadOrCreate(home);
  const trusted = new TrustedPeers(home);

  const ws = await connectWebSocket(`ws://${host}:${port}`);
  let session;
  try {
    session = await pairClient(ws, identity, code);
  } catch (e) {
    await ws.close().catch(() => {});
    throw e;
  }

  const serverPubkey = session.peerPublic;
  const peerName = name || `peer-${serverPubkey.toString("hex").slice(0, 6)}`;

  // Cache endpoint before HELLO so reconnect logic can find the peer.
  trusted.cacheEndpoint(serverPubkey, host, port);

  // Complete the HELLO exchange first — on same-machine tests the server writes
  // its own auto-generated name to the shared trusted-peers file during enablePairing
  // (before HELLO). Writing our name after HELLO guarantees we win that race.
  const remote = await RemoteComputer.connect(session);
  trusted.add(serverPubkey, { name: peerName });
  return { remote, serverPubkey };
}

// ---------------------------------------------------------------------------
// Internal: resolve a target to (host, port, pubkey)
// ---------------------------------------------------------------------------

async function resolve(
  target: Target,
  opts: { home?: string; timeout?: number },
): Promise<{ host: string; port: number; pubkey: Buffer }> {
  // DiscoveredPeer object
  if (typeof target === "object") {
    return { host: target.host, port: target.port, pubkey: target.publicKey };
  }

  // Explicit ws:// URL with #pubkey-hex fragment
  if (target.startsWith("ws://") || target.startsWith("wss://")) {
    const [url, frag] = target.split("#");
    if (!frag) throw new Error(`explicit URL requires '#<pubkey-hex>' fragment: ${target}`);
    const pubkey = Buffer.from(frag, "hex");
    const hostPort = url.replace(/^wss?:\/\//, "");
    const colonIdx = hostPort.lastIndexOf(":");
    const host = hostPort.slice(0, colonIdx);
    const port = parseInt(hostPort.slice(colonIdx + 1), 10) || 80;
    return { host, port, pubkey };
  }

  // Peer name — look up in trusted-peers
  const store = new TrustedPeers(opts.home);
  const peer = store.findByName(target);
  if (!peer) {
    throw new Error(`unknown peer '${target}'; run 'opendesk pair-with <host> <code>' first`);
  }
  const pubkey = Buffer.from(peer.publicKey, "hex");

  // Use cached endpoint if available
  if (peer.lastHost && peer.lastPort) {
    return { host: peer.lastHost, port: peer.lastPort, pubkey };
  }

  // Fall back to mDNS discovery
  const { discover } = await import("./discovery.js");
  const peers = await discover(opts.timeout ?? 5000);
  for (const p of peers) {
    if (p.publicKey.equals(pubkey)) {
      return { host: p.host, port: p.port, pubkey };
    }
  }

  throw new Error(
    `peer '${target}' is paired but has no cached address and could not be found via mDNS`,
  );
}
