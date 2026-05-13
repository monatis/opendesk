/**
 * Long-lived peer identity — an X25519 keypair persisted on disk.
 * Mirrors Python opendesk.protocol.auth.identity.
 *
 * Uses Node.js built-in crypto (requires Node 18+).
 */

import crypto from "crypto";
import fs from "fs";
import os from "os";
import path from "path";

export const DEFAULT_HOME = path.join(os.homedir(), ".opendesk");
export const IDENTITY_FILE = "identity.key";
const KEY_BYTES = 32;

// DER prefix for X25519 PKCS8 private key (RFC 8410)
const PKCS8_PREFIX = Buffer.from("302e020100300506032b656e04220420", "hex");
// DER prefix for X25519 SubjectPublicKeyInfo
const SPKI_PREFIX = Buffer.from("302a300506032b656e032100", "hex");

export function rawPubFromPriv(privBytes: Buffer): Buffer {
  const privKey = privKeyFromBytes(privBytes);
  const pubKey = crypto.createPublicKey(privKey);
  const jwk = pubKey.export({ format: "jwk" }) as crypto.JsonWebKey;
  return Buffer.from(jwk.x!, "base64url");
}

export function privKeyFromBytes(bytes: Buffer): crypto.KeyObject {
  return crypto.createPrivateKey({
    key: Buffer.concat([PKCS8_PREFIX, bytes]),
    format: "der",
    type: "pkcs8",
  });
}

export function pubKeyFromBytes(bytes: Buffer): crypto.KeyObject {
  return crypto.createPublicKey({
    key: Buffer.concat([SPKI_PREFIX, bytes]),
    format: "der",
    type: "spki",
  });
}

export function dhExchange(privBytes: Buffer, peerPubBytes: Buffer): Buffer {
  const privKey = privKeyFromBytes(privBytes);
  const pubKey = pubKeyFromBytes(peerPubBytes);
  return crypto.diffieHellman({ privateKey: privKey, publicKey: pubKey });
}

export class Identity {
  readonly privateBytes: Buffer;
  readonly publicBytes: Buffer;

  constructor(privateBytes: Buffer) {
    if (privateBytes.length !== KEY_BYTES) {
      throw new Error(`identity key must be ${KEY_BYTES} bytes`);
    }
    this.privateBytes = privateBytes;
    this.publicBytes = rawPubFromPriv(privateBytes);
  }

  static generate(): Identity {
    const { privateKey } = crypto.generateKeyPairSync("x25519");
    const jwk = privateKey.export({ format: "jwk" }) as crypto.JsonWebKey;
    return new Identity(Buffer.from(jwk.d!, "base64url"));
  }

  static fromPrivateBytes(bytes: Buffer): Identity {
    return new Identity(bytes);
  }

  exchange(peerPublicBytes: Buffer): Buffer {
    return dhExchange(this.privateBytes, peerPublicBytes);
  }

  // ------------------------------------------------------------------
  // Persistence
  // ------------------------------------------------------------------

  static loadOrCreate(home?: string): Identity {
    const homeDir = home ?? DEFAULT_HOME;
    const keyPath = path.join(homeDir, IDENTITY_FILE);
    if (fs.existsSync(keyPath)) {
      return new Identity(fs.readFileSync(keyPath));
    }
    fs.mkdirSync(homeDir, { recursive: true, mode: 0o700 });
    const identity = Identity.generate();
    atomicWriteSecret(keyPath, identity.privateBytes);
    return identity;
  }

  save(home?: string): void {
    const homeDir = home ?? DEFAULT_HOME;
    fs.mkdirSync(homeDir, { recursive: true, mode: 0o700 });
    atomicWriteSecret(path.join(homeDir, IDENTITY_FILE), this.privateBytes);
  }
}

function atomicWriteSecret(filePath: string, data: Buffer): void {
  const tmp = filePath + ".tmp." + process.pid;
  try {
    fs.writeFileSync(tmp, data, { mode: 0o600 });
    fs.renameSync(tmp, filePath);
  } catch (err) {
    try { fs.unlinkSync(tmp); } catch { /* ignore */ }
    throw err;
  }
}

export function generatePairingCode(digits = 6): string {
  const max = 10 ** digits;
  const n = crypto.randomInt(max);
  return String(n).padStart(digits, "0");
}

export function fingerprint(publicKey: Buffer): string {
  const h = publicKey.toString("hex");
  return [h.slice(0, 4), h.slice(4, 8), h.slice(8, 12), h.slice(12, 16)].join(":");
}
