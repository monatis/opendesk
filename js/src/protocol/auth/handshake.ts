/**
 * Mutual-authentication handshake — mirrors Python opendesk.protocol.auth.handshake.
 *
 * Two flavours:
 *   pairServer / pairClient   — first contact, PSK-authenticated (6-digit code)
 *   authServer / authClient   — subsequent connections, static-key authenticated
 *
 * Both produce a Session with an EncryptedConnection ready for use.
 *
 * Wire messages are msgpack-encoded dicts (NOT the main frame codec).
 */

import crypto from "crypto";
import { pack, unpack } from "msgpackr";
import { Connection, ConnectionClosed } from "../connection.js";
import { EncryptedConnection } from "./encrypted.js";
import { Identity, dhExchange, privKeyFromBytes } from "./identity.js";
import type { TrustedPeers } from "./storage.js";

export const HANDSHAKE_VERSION = 1;
const PSK_SALT = Buffer.from("opendesk-psk-v1");
const PSK_ITERATIONS = 200_000;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

export class AuthFailure extends Error {
  reason: string;
  constructor(reason: string, message = "") {
    super(message ? `${reason}: ${message}` : reason);
    this.name = "AuthFailure";
    this.reason = reason;
  }
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

export interface Session {
  connection: EncryptedConnection;
  peerPublic: Buffer;
  isPairing: boolean;
}

// ---------------------------------------------------------------------------
// Crypto helpers
// ---------------------------------------------------------------------------

export function derivePsk(code: string): Buffer {
  return crypto.pbkdf2Sync(Buffer.from(code, "utf8"), PSK_SALT, PSK_ITERATIONS, 32, "sha256");
}

function hkdf(salt: Buffer, ikm: Buffer, info: Buffer, length = 32): Buffer {
  return Buffer.from(crypto.hkdfSync("sha256", ikm, salt, info, length));
}

function genEphemeral(): { privBytes: Buffer; pubBytes: Buffer } {
  const { privateKey } = crypto.generateKeyPairSync("x25519");
  const jwk = privateKey.export({ format: "jwk" }) as crypto.JsonWebKey;
  const privBytes = Buffer.from(jwk.d!, "base64url");
  const pubBytes = Buffer.from(jwk.x!, "base64url");
  return { privBytes, pubBytes };
}

function chacha20Seal(key: Buffer, plaintext: Buffer): Buffer {
  const nonce = Buffer.alloc(12);
  const cipher = crypto.createCipheriv("chacha20-poly1305", key, nonce, {
    authTagLength: 16,
  } as Parameters<typeof crypto.createCipheriv>[3]);
  const enc = Buffer.concat([cipher.update(plaintext), cipher.final()]);
  const tag = (cipher as crypto.CipherGCM).getAuthTag();
  return Buffer.concat([enc, tag]);
}

function chacha20Open(key: Buffer, ciphertext: Buffer): Buffer {
  const nonce = Buffer.alloc(12);
  const data = ciphertext.subarray(0, -16);
  const tag = ciphertext.subarray(-16);
  const decipher = crypto.createDecipheriv("chacha20-poly1305", key, nonce, {
    authTagLength: 16,
  } as Parameters<typeof crypto.createDecipheriv>[3]);
  (decipher as crypto.DecipherGCM).setAuthTag(tag);
  try {
    return Buffer.concat([decipher.update(data), decipher.final()]);
  } catch {
    throw new AuthFailure("wrong_code", "AEAD decryption failed");
  }
}

function sessionKeys(
  isServer: boolean,
  transcript: Buffer,
  dhChain: Buffer[],
  psk?: Buffer,
): { sendKey: Buffer; recvKey: Buffer } {
  let ikm = Buffer.concat(dhChain);
  if (psk) ikm = Buffer.concat([ikm, psk]);
  const keys = hkdf(transcript, ikm, Buffer.from("opendesk-session-keys"), 64);
  const c2s = keys.subarray(0, 32);
  const s2c = keys.subarray(32, 64);
  return isServer
    ? { sendKey: s2c, recvKey: c2s }
    : { sendKey: c2s, recvKey: s2c };
}

// ---------------------------------------------------------------------------
// Wire helpers
// ---------------------------------------------------------------------------

async function sendMsg(conn: Connection, payload: Record<string, unknown>): Promise<void> {
  await conn.send(pack({ v: HANDSHAKE_VERSION, ...payload }) as Buffer);
}

async function recvMsg(conn: Connection, expectedKind: string): Promise<Record<string, unknown>> {
  let data: Buffer;
  try {
    data = await conn.recv();
  } catch (e) {
    throw new AuthFailure("protocol", `peer closed before ${expectedKind}`);
  }
  let msg: unknown;
  try {
    msg = unpack(data);
  } catch {
    throw new AuthFailure("protocol", `malformed ${expectedKind} msgpack`);
  }
  if (typeof msg !== "object" || msg === null || Array.isArray(msg)) {
    throw new AuthFailure("protocol", `${expectedKind} not an object`);
  }
  const m = msg as Record<string, unknown>;
  if (m["v"] !== HANDSHAKE_VERSION) {
    throw new AuthFailure("protocol", `unsupported handshake version ${m["v"]}`);
  }
  if (m["kind"] !== expectedKind) {
    throw new AuthFailure("protocol", `expected ${expectedKind}, got ${m["kind"]}`);
  }
  return m;
}

function requireBytes(msg: Record<string, unknown>, field: string, length?: number): Buffer {
  const val = msg[field];
  if (!(val instanceof Buffer) && !(val instanceof Uint8Array)) {
    throw new AuthFailure("protocol", `field '${field}' missing or wrong type`);
  }
  const b = Buffer.from(val as Uint8Array);
  if (length !== undefined && b.length !== length) {
    throw new AuthFailure("protocol", `field '${field}' must be ${length} bytes, got ${b.length}`);
  }
  return b;
}

// ---------------------------------------------------------------------------
// Pairing — server side
// ---------------------------------------------------------------------------

export async function pairServer(
  connection: Connection,
  identity: Identity,
  code: string,
): Promise<Session> {
  const psk = derivePsk(code);
  const { privBytes: ephPriv, pubBytes: ephPub } = genEphemeral();

  const msg1 = await recvMsg(connection, "pair_hello");
  const eC = requireBytes(msg1, "e", 32);

  const dh_ee = dhExchange(ephPriv, eC);
  const k1 = hkdf(psk, dh_ee, Buffer.from("opendesk-pair-k1"));
  const ct1 = chacha20Seal(k1, identity.publicBytes);
  await sendMsg(connection, { kind: "pair_offer", e: ephPub, ct: ct1 });

  const msg3 = await recvMsg(connection, "pair_finish");
  const ct2 = requireBytes(msg3, "ct");

  const k2 = hkdf(Buffer.concat([psk, ct1]), dh_ee, Buffer.from("opendesk-pair-k2"));
  const clientStatic = chacha20Open(k2, ct2);
  if (clientStatic.length !== 32) {
    throw new AuthFailure("protocol", "decrypted client static key wrong length");
  }

  const transcript = crypto.createHash("sha256")
    .update(ephPub).update(eC).update(ct1).update(ct2)
    .digest();
  const dhChain = [
    dh_ee,
    identity.exchange(eC),
    dhExchange(ephPriv, clientStatic),
  ];
  const { sendKey, recvKey } = sessionKeys(true, transcript, dhChain, psk);

  return {
    connection: new EncryptedConnection(connection, { sendKey, recvKey, peerPublic: clientStatic }),
    peerPublic: clientStatic,
    isPairing: true,
  };
}

// ---------------------------------------------------------------------------
// Pairing — client side
// ---------------------------------------------------------------------------

export async function pairClient(
  connection: Connection,
  identity: Identity,
  code: string,
): Promise<Session> {
  const psk = derivePsk(code);
  const { privBytes: ephPriv, pubBytes: ephPub } = genEphemeral();

  await sendMsg(connection, { kind: "pair_hello", e: ephPub });

  const msg2 = await recvMsg(connection, "pair_offer");
  const eS = requireBytes(msg2, "e", 32);
  const ct1 = requireBytes(msg2, "ct");

  const dh_ee = dhExchange(ephPriv, eS);
  const k1 = hkdf(psk, dh_ee, Buffer.from("opendesk-pair-k1"));
  const serverStatic = chacha20Open(k1, ct1);
  if (serverStatic.length !== 32) {
    throw new AuthFailure("protocol", "decrypted server static key wrong length");
  }

  const k2 = hkdf(Buffer.concat([psk, ct1]), dh_ee, Buffer.from("opendesk-pair-k2"));
  const ct2 = chacha20Seal(k2, identity.publicBytes);
  await sendMsg(connection, { kind: "pair_finish", ct: ct2 });

  const transcript = crypto.createHash("sha256")
    .update(eS).update(ephPub).update(ct1).update(ct2)
    .digest();
  const dhChain = [
    dh_ee,
    dhExchange(ephPriv, serverStatic),
    identity.exchange(eS),
  ];
  const { sendKey, recvKey } = sessionKeys(false, transcript, dhChain, psk);

  return {
    connection: new EncryptedConnection(connection, { sendKey, recvKey, peerPublic: serverStatic }),
    peerPublic: serverStatic,
    isPairing: true,
  };
}

// ---------------------------------------------------------------------------
// Reconnect — server side
// ---------------------------------------------------------------------------

export async function authServer(
  connection: Connection,
  identity: Identity,
  trusted: TrustedPeers,
): Promise<Session> {
  const { privBytes: ephPriv, pubBytes: ephPub } = genEphemeral();

  const msg1 = await recvMsg(connection, "auth_hello");
  const eC = requireBytes(msg1, "e", 32);
  const sC = requireBytes(msg1, "s", 32);

  if (!trusted.contains(sC)) {
    await sendMsg(connection, { kind: "auth_offer", e: ephPub, ct: Buffer.alloc(0) });
    throw new AuthFailure("untrusted_peer", "client static key not in trusted-peers");
  }

  const dhChain = [
    dhExchange(ephPriv, eC),
    identity.exchange(eC),
    dhExchange(ephPriv, sC),
    identity.exchange(sC),
  ];
  const transcript = crypto.createHash("sha256")
    .update(ephPub).update(eC).update(sC)
    .digest();
  const k = hkdf(transcript, Buffer.concat(dhChain.slice(0, 2)), Buffer.from("opendesk-auth-k"));
  const ct = chacha20Seal(k, Buffer.from("ok"));
  await sendMsg(connection, { kind: "auth_offer", e: ephPub, ct });

  const { sendKey, recvKey } = sessionKeys(true, transcript, dhChain);
  return {
    connection: new EncryptedConnection(connection, { sendKey, recvKey, peerPublic: sC }),
    peerPublic: sC,
    isPairing: false,
  };
}

// ---------------------------------------------------------------------------
// Reconnect — client side
// ---------------------------------------------------------------------------

export async function authClient(
  connection: Connection,
  identity: Identity,
  expectedServerPubkey: Buffer,
): Promise<Session> {
  if (expectedServerPubkey.length !== 32) {
    throw new Error("expectedServerPubkey must be 32 bytes");
  }
  const { privBytes: ephPriv, pubBytes: ephPub } = genEphemeral();

  await sendMsg(connection, {
    kind: "auth_hello",
    e: ephPub,
    s: identity.publicBytes,
  });

  const msg2 = await recvMsg(connection, "auth_offer");
  const eS = requireBytes(msg2, "e", 32);
  const ct = requireBytes(msg2, "ct");

  const dhChain = [
    dhExchange(ephPriv, eS),
    dhExchange(ephPriv, expectedServerPubkey),
    identity.exchange(eS),
    identity.exchange(expectedServerPubkey),
  ];
  const transcript = crypto.createHash("sha256")
    .update(eS).update(ephPub).update(identity.publicBytes)
    .digest();
  const k = hkdf(transcript, Buffer.concat(dhChain.slice(0, 2)), Buffer.from("opendesk-auth-k"));
  const ok = chacha20Open(k, ct);
  if (ok.toString() !== "ok") {
    throw new AuthFailure("unexpected_peer", "server confirmation payload mismatch");
  }

  const { sendKey, recvKey } = sessionKeys(false, transcript, dhChain);
  return {
    connection: new EncryptedConnection(connection, { sendKey, recvKey, peerPublic: expectedServerPubkey }),
    peerPublic: expectedServerPubkey,
    isPairing: false,
  };
}
