/**
 * msgpack codec for opendesk wire frames.
 *
 * Buffers flow as msgpack bin (never base64).  Uses msgpackr which maps
 * Node.js Buffer / Uint8Array ↔ msgpack bin natively.
 */

import { pack, unpack } from "msgpackr";
import type { Frame } from "./frames.js";

export class CodecError extends Error {}

export function encode(frame: Frame): Buffer {
  try {
    return pack(frame) as Buffer;
  } catch (err) {
    throw new CodecError(`msgpack pack failed: ${err}`);
  }
}

export function decode(data: Buffer): Frame {
  let obj: unknown;
  try {
    obj = unpack(data);
  } catch (err) {
    throw new CodecError(`msgpack unpack failed: ${err}`);
  }
  if (typeof obj !== "object" || obj === null || Array.isArray(obj)) {
    throw new CodecError(`frame must decode to an object`);
  }
  const raw = obj as Record<string, unknown>;
  const type = raw["type"];
  if (typeof type !== "string") {
    throw new CodecError(`missing or invalid 'type' field: ${JSON.stringify(type)}`);
  }
  const allowed = ["hello", "req", "res", "cancel", "push"];
  if (!allowed.includes(type)) {
    throw new CodecError(`unknown frame type: ${type}`);
  }
  return raw as unknown as Frame;
}
