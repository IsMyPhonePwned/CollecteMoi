import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";

import { generateDeviceKeyPair } from "./keys.js";
import { generateNoiseKey, sealNoise } from "./noise.js";
import type { Session } from "./session.js";

const DEFAULT_CHUNK = 8 * 1024 * 1024;

export type Artefact =
  | "android_bugreport"
  | "ios_sysdiagnose"
  | "message_report";

export type CollectionRequest = {
  id: string;
  case_id: string;
  title: string;
  artefacts: string[];
  deadline?: string | null;
};

export type ClientSubmission = {
  upload_id: string;
  artefact: Artefact;
  size_bytes: number;
  submitted_at: string;
  status: string;
};

export type EnrollResult = Session;

async function jsonFetch<T>(
  url: string,
  init?: RequestInit & { expect?: number[] },
): Promise<T> {
  const res = await fetch(url, init);
  const text = await res.text();
  const expect = init?.expect ?? [200, 201, 202, 204];
  if (!expect.includes(res.status)) {
    throw new Error(`${init?.method ?? "GET"} ${url} → ${res.status}: ${text}`);
  }
  if (!text || res.status === 204) return undefined as T;
  return JSON.parse(text) as T;
}

export class CollecteMoiClient {
  constructor(
    public server: string,
    public appVersion = "0.1.0",
  ) {
    this.server = server.replace(/\/$/, "");
  }

  private deviceHeaders(
    session?: Session | null,
    extra?: {
      platform?: string;
      model?: string;
      osVersion?: string;
    },
  ): Record<string, string> {
    const platform = extra?.platform ?? "desktop";
    const model = extra?.model ?? process.platform;
    const osVersion = extra?.osVersion ?? process.version;
    const headers: Record<string, string> = {
      "user-agent": `CollecteMoi/${this.appVersion} (${platform}; ${model})`,
      "x-app-version": this.appVersion,
      "x-device-platform": platform,
      "x-device-model": model,
      "x-device-os": osVersion,
    };
    if (session?.deviceId) headers["x-device-id"] = session.deviceId;
    return headers;
  }

  async healthz(): Promise<string> {
    const res = await fetch(`${this.server}/healthz`);
    return res.text();
  }

  async manifest(): Promise<Record<string, unknown>> {
    return jsonFetch(`${this.server}/v1/tenant/manifest`);
  }

  async requests(session?: Session | null): Promise<CollectionRequest[]> {
    return jsonFetch(`${this.server}/v1/requests`, {
      headers: this.deviceHeaders(session),
    });
  }

  async submissions(session: Session): Promise<ClientSubmission[]> {
    const proof = sealNoise(
      Buffer.from(`collectemoi-client-submissions-v1:${session.deviceId}`),
      session.noiseKeyHex,
    );
    return jsonFetch<ClientSubmission[]>(`${this.server}/v1/client/submissions`, {
      method: "POST",
      headers: {
        "content-type": "application/octet-stream",
        ...this.deviceHeaders(session),
      },
      body: proof,
    });
  }

  /** Email OTP step 1 (or lab_key shortcut). */
  async enrollStart(opts: {
    email?: string;
    labKey?: string;
  }): Promise<{
    challengeId: string;
    email: string;
    emailMasked: string;
    companyId: string;
    companyName: string;
    expiresInSecs: number;
    labOtp?: string;
    message: string;
  }> {
    const body = await jsonFetch<{
      challenge_id: string;
      email: string;
      email_masked: string;
      company_id: string;
      company_name: string;
      expires_in_secs: number;
      lab_otp?: string;
      message: string;
    }>(`${this.server}/v1/enroll/start`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...this.deviceHeaders(),
      },
      body: JSON.stringify({
        email: opts.email,
        lab_key: opts.labKey,
      }),
    });
    return {
      challengeId: body.challenge_id,
      email: body.email,
      emailMasked: body.email_masked,
      companyId: body.company_id,
      companyName: body.company_name,
      expiresInSecs: body.expires_in_secs,
      labOtp: body.lab_otp,
      message: body.message,
    };
  }

  /** Email OTP step 2 — confirm code + exchange device/noise keys. */
  async enrollConfirm(opts: {
    challengeId: string;
    code: string;
    platform?: string;
    model?: string;
    osVersion?: string;
    email?: string;
    companyId?: string;
  }): Promise<EnrollResult> {
    const keys = generateDeviceKeyPair();
    const noiseKeyHex = generateNoiseKey();
    const body = await jsonFetch<{
      device_id: string;
      case_id: string;
      case_public_key: string;
      server_base_url: string;
      display_name: string;
      enabled_collectors: string[];
      noise_key_fingerprint: string;
    }>(`${this.server}/v1/enroll/confirm`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...this.deviceHeaders(null, {
          platform: opts.platform,
          model: opts.model,
          osVersion: opts.osVersion,
        }),
      },
      body: JSON.stringify({
        challenge_id: opts.challengeId,
        code: opts.code,
        device_public_key: keys.publicKeyHex,
        noise_key: noiseKeyHex,
        platform: opts.platform ?? "desktop",
        model: opts.model ?? process.platform,
        os_version: opts.osVersion ?? process.version,
        app_version: this.appVersion,
      }),
      expect: [201],
    });

    return {
      serverBaseUrl: body.server_base_url || this.server,
      deviceId: body.device_id,
      caseId: body.case_id,
      casePublicKeyHex: body.case_public_key,
      devicePublicKeyHex: keys.publicKeyHex,
      devicePrivateKeyHex: keys.privateKeyHex,
      noiseKeyHex,
      noiseKeyFingerprint: body.noise_key_fingerprint,
      displayName: body.display_name,
      enabledCollectors: body.enabled_collectors ?? [],
      enrolledAt: new Date().toISOString(),
      email: opts.email,
      companyId: opts.companyId,
    };
  }

  /** Full email/lab enroll helper. */
  async enrollWithEmail(opts: {
    email?: string;
    labKey?: string;
    code?: string;
    platform?: string;
    model?: string;
    osVersion?: string;
  }): Promise<EnrollResult & { labOtp?: string }> {
    const started = await this.enrollStart({
      email: opts.email,
      labKey: opts.labKey,
    });
    const code = opts.code || started.labOtp;
    if (!code) {
      throw new Error(
        "OTP required — pass --code, or use lab mode (server returns lab_otp)",
      );
    }
    const session = await this.enrollConfirm({
      challengeId: started.challengeId,
      code,
      platform: opts.platform,
      model: opts.model,
      osVersion: opts.osVersion,
      email: started.email,
      companyId: started.companyId,
    });
    return { ...session, labOtp: started.labOtp };
  }

  /** Legacy token enroll (still supported). */
  async enroll(opts: {
    token: string;
    platform?: string;
    model?: string;
    osVersion?: string;
  }): Promise<EnrollResult> {
    const keys = generateDeviceKeyPair();
    const noiseKeyHex = generateNoiseKey();
    const body = await jsonFetch<{
      device_id: string;
      case_id: string;
      case_public_key: string;
      server_base_url: string;
      display_name: string;
      enabled_collectors: string[];
      noise_key_fingerprint: string;
    }>(`${this.server}/v1/enroll`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...this.deviceHeaders(null, {
          platform: opts.platform,
          model: opts.model,
          osVersion: opts.osVersion,
        }),
      },
      body: JSON.stringify({
        token: opts.token,
        device_public_key: keys.publicKeyHex,
        noise_key: noiseKeyHex,
        platform: opts.platform ?? "desktop",
        model: opts.model ?? process.platform,
        os_version: opts.osVersion ?? process.version,
        app_version: this.appVersion,
      }),
      expect: [201],
    });

    return {
      serverBaseUrl: body.server_base_url || this.server,
      deviceId: body.device_id,
      caseId: body.case_id,
      casePublicKeyHex: body.case_public_key,
      devicePublicKeyHex: keys.publicKeyHex,
      devicePrivateKeyHex: keys.privateKeyHex,
      noiseKeyHex,
      noiseKeyFingerprint: body.noise_key_fingerprint,
      displayName: body.display_name,
      enabledCollectors: body.enabled_collectors ?? [],
      enrolledAt: new Date().toISOString(),
    };
  }

  /** Seal plaintext via server lab endpoint (same CM01 as Flutter web). */
  async labSeal(opts: {
    plaintext: Uint8Array;
    artefact: Artefact;
    casePublicKeyHex: string;
    devicePrivateKeyHex: string;
  }): Promise<Uint8Array> {
    const body = await jsonFetch<{ envelope_b64: string }>(
      `${this.server}/v1/lab/seal`,
      {
        method: "POST",
        headers: {
          "content-type": "application/json",
          ...this.deviceHeaders(),
        },
        body: JSON.stringify({
          plaintext_b64: Buffer.from(opts.plaintext).toString("base64"),
          artefact: opts.artefact,
          case_public_key: opts.casePublicKeyHex,
          device_private_key: opts.devicePrivateKeyHex,
        }),
      },
    );
    return Buffer.from(body.envelope_b64, "base64");
  }

  async uploadEnvelope(
    session: Session,
    envelope: Uint8Array,
    artefact: Artefact,
    chunkSize = DEFAULT_CHUNK,
  ): Promise<{ uploadId: string }> {
    if (!session.noiseKeyHex) {
      throw new Error("session missing noiseKeyHex — re-enroll");
    }
    // Wrap CM01 with the per-client ChaCha20 noise key before chunking.
    const wrapped = sealNoise(envelope, session.noiseKeyHex);
    const totalSize = wrapped.byteLength;
    const chunkCount = Math.max(1, Math.ceil(totalSize / chunkSize));
    const ciphertextSha256 = createHash("sha256").update(wrapped).digest("hex");

    const deviceHdrs = this.deviceHeaders(session);
    const created = await jsonFetch<{ upload_id: string }>(
      `${this.server}/v1/uploads`,
      {
        method: "POST",
        headers: { "content-type": "application/json", ...deviceHdrs },
        body: JSON.stringify({
          case_id: session.caseId,
          artefact,
          total_size: totalSize,
          chunk_size: chunkSize,
          chunk_count: chunkCount,
          ciphertext_sha256: ciphertextSha256,
        }),
        expect: [201],
      },
    );

    for (let n = 0; n < chunkCount; n++) {
      const start = n * chunkSize;
      const end = Math.min(start + chunkSize, totalSize);
      const chunk = wrapped.subarray(start, end);
      const digest = createHash("sha256").update(chunk).digest("hex");
      await jsonFetch(
        `${this.server}/v1/uploads/${created.upload_id}/chunks/${n}`,
        {
          method: "PUT",
          headers: {
            "content-type": "application/octet-stream",
            digest: `sha-256=${digest}`,
            ...deviceHdrs,
          },
          body: Buffer.from(chunk),
          expect: [204, 200],
        },
      );
    }

    await jsonFetch(`${this.server}/v1/uploads/${created.upload_id}/complete`, {
      method: "POST",
      headers: { "content-type": "application/json", ...deviceHdrs },
      body: JSON.stringify({
        device_public_key: session.devicePublicKeyHex,
      }),
    });

    return { uploadId: created.upload_id };
  }

  async uploadFile(
    session: Session,
    filePath: string,
    artefact: Artefact = "android_bugreport",
  ): Promise<{ uploadId: string; bytes: number; envelopeBytes: number }> {
    const plaintext = await readFile(filePath);
    const envelope = await this.labSeal({
      plaintext,
      artefact,
      casePublicKeyHex: session.casePublicKeyHex,
      devicePrivateKeyHex: session.devicePrivateKeyHex,
    });
    const { uploadId } = await this.uploadEnvelope(session, envelope, artefact);
    return {
      uploadId,
      bytes: plaintext.byteLength,
      envelopeBytes: envelope.byteLength,
    };
  }
}
