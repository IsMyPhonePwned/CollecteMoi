import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname } from "node:path";

export type Session = {
  serverBaseUrl: string;
  deviceId: string;
  caseId: string;
  casePublicKeyHex: string;
  devicePublicKeyHex: string;
  devicePrivateKeyHex: string;
  /** ChaCha20-Poly1305 noise key exchanged at enroll (32 bytes hex). */
  noiseKeyHex: string;
  noiseKeyFingerprint?: string;
  displayName: string;
  enabledCollectors: string[];
  enrolledAt: string;
  email?: string;
  companyId?: string;
};

export async function loadSession(path: string): Promise<Session> {
  const raw = await readFile(path, "utf8");
  return JSON.parse(raw) as Session;
}

export async function saveSession(path: string, session: Session): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, JSON.stringify(session, null, 2) + "\n", {
    mode: 0o600,
  });
}
