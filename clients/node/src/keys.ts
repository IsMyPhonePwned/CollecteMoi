import { p256 } from "@noble/curves/p256";

export type DeviceKeyPair = {
  privateKeyHex: string;
  publicKeyHex: string;
};

function toHex(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("hex");
}

/** P-256 device key: 32-byte sk + uncompressed SEC1 pk (65 bytes, 0x04||X||Y). */
export function generateDeviceKeyPair(): DeviceKeyPair {
  const sk = p256.utils.randomPrivateKey();
  const pk = p256.getPublicKey(sk, false);
  return {
    privateKeyHex: toHex(sk),
    publicKeyHex: toHex(pk),
  };
}
