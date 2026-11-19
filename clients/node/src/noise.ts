import { createCipheriv, createDecipheriv, createHash, randomBytes } from "node:crypto";

const MAGIC = Buffer.from("CMNK");
const KEY_LEN = 32;
const NONCE_LEN = 12;
const TAG_LEN = 16;

export function generateNoiseKey(): string {
  return randomBytes(KEY_LEN).toString("hex");
}

export function noiseFingerprint(keyHex: string): string {
  return createHash("sha256").update(Buffer.from(keyHex, "hex")).digest("hex");
}

/** Wrap CM01 envelope: CMNK || nonce(12) || ciphertext||tag */
export function sealNoise(plaintext: Uint8Array, keyHex: string): Buffer {
  const key = Buffer.from(keyHex, "hex");
  if (key.length !== KEY_LEN) throw new Error("noise key must be 32 bytes");
  const nonce = randomBytes(NONCE_LEN);
  const cipher = createCipheriv("chacha20-poly1305", key, nonce, {
    authTagLength: TAG_LEN,
  });
  const ct = Buffer.concat([cipher.update(Buffer.from(plaintext)), cipher.final()]);
  const tag = cipher.getAuthTag();
  return Buffer.concat([MAGIC, nonce, ct, tag]);
}

export function openNoise(blob: Uint8Array, keyHex: string): Buffer {
  const key = Buffer.from(keyHex, "hex");
  const buf = Buffer.from(blob);
  if (buf.length < MAGIC.length + NONCE_LEN + TAG_LEN) {
    throw new Error("noise blob too short");
  }
  if (!buf.subarray(0, 4).equals(MAGIC)) throw new Error("not CMNK");
  const nonce = buf.subarray(4, 4 + NONCE_LEN);
  const tag = buf.subarray(buf.length - TAG_LEN);
  const ct = buf.subarray(4 + NONCE_LEN, buf.length - TAG_LEN);
  const decipher = createDecipheriv("chacha20-poly1305", key, nonce, {
    authTagLength: TAG_LEN,
  });
  decipher.setAuthTag(tag);
  return Buffer.concat([decipher.update(ct), decipher.final()]);
}
