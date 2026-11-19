#!/usr/bin/env node
import { resolve } from "node:path";

import { CollecteMoiClient, type Artefact } from "./client.js";
import { loadSession, saveSession } from "./session.js";
import { configureTls } from "./tls.js";

type Args = Record<string, string | boolean>;

function parseArgs(argv: string[]): { cmd: string; args: Args } {
  const [cmd = "help", ...rest] = argv;
  const args: Args = {};
  for (let i = 0; i < rest.length; i++) {
    const a = rest[i]!;
    if (!a.startsWith("--")) continue;
    const key = a.slice(2);
    const next = rest[i + 1];
    if (!next || next.startsWith("--")) {
      args[key] = true;
    } else {
      args[key] = next;
      i++;
    }
  }
  return { cmd, args };
}

function str(args: Args, key: string, fallback?: string): string {
  const v = args[key];
  if (typeof v === "string" && v.length) return v;
  if (fallback !== undefined) return fallback;
  throw new Error(`missing --${key}`);
}

function opt(args: Args, key: string): string | undefined {
  const v = args[key];
  return typeof v === "string" && v.length ? v : undefined;
}

function usage(): never {
  console.log(`CollecteMoi desktop client

Usage:
  collectemoi enroll   --server <url> (--email <addr> | --lab-key <key>) [--code <otp>]
  collectemoi enroll   --server <url> --token <legacy-token>   # legacy
  collectemoi requests --session <path>
  collectemoi upload   --session <path> --file <path> [--artefact <kind>]
  collectemoi collect  --session <path> --file <path> [--artefact <kind>]

Email OTP:
  1) start with --email user@example.com  (domain must belong to a company)
  2) confirm with --code <6-digit> (lab servers return lab_otp automatically)

Lab quick path:
  --lab-key LAB-DEMO01   (OTP returned as lab_otp; key itself also works as code)

TLS (lab self-signed):
  --insecure           Skip TLS certificate verification
  --tls-ca <pem>       Trust this CA/cert PEM instead

Default session: ./.collectemoi/session.json
Default server: https://127.0.0.1:8080
`);
  process.exit(1);
}

async function main() {
  const { cmd, args } = parseArgs(process.argv.slice(2));
  const sessionPath = resolve(
    str(args, "session", "./.collectemoi/session.json"),
  );

  if (cmd === "help" || args.help === true) usage();

  configureTls({
    insecure: args.insecure === true,
    tlsCa: typeof args["tls-ca"] === "string" ? args["tls-ca"] : undefined,
  });

  if (cmd === "enroll") {
    const server = str(args, "server", "https://127.0.0.1:8080");
    const client = new CollecteMoiClient(server);
    const legacyToken = opt(args, "token");
    let session;
    if (legacyToken) {
      session = await client.enroll({ token: legacyToken });
    } else {
      session = await client.enrollWithEmail({
        email: opt(args, "email"),
        labKey: opt(args, "lab-key"),
        code: opt(args, "code"),
      });
    }
    await saveSession(sessionPath, session);
    console.log(
      JSON.stringify(
        {
          session: sessionPath,
          deviceId: session.deviceId,
          caseId: session.caseId,
          email: session.email,
          companyId: session.companyId,
          displayName: session.displayName,
          server: session.serverBaseUrl,
          labOtp: "labOtp" in session ? (session as { labOtp?: string }).labOtp : undefined,
        },
        null,
        2,
      ),
    );
    return;
  }

  if (cmd === "requests") {
    const session = await loadSession(sessionPath);
    const client = new CollecteMoiClient(session.serverBaseUrl);
    const list = await client.requests();
    console.log(JSON.stringify(list, null, 2));
    return;
  }

  if (cmd === "upload" || cmd === "collect") {
    const session = await loadSession(sessionPath);
    const client = new CollecteMoiClient(session.serverBaseUrl);
    const file = resolve(str(args, "file"));
    const artefact = str(args, "artefact", "android_bugreport") as Artefact;

    if (cmd === "collect") {
      const list = await client.requests();
      console.log("Open collection requests:");
      console.log(JSON.stringify(list, null, 2));
      const allowed = new Set(list.flatMap((r) => r.artefacts));
      if (allowed.size && !allowed.has(artefact)) {
        throw new Error(
          `artefact ${artefact} not in open requests (${[...allowed].join(", ")})`,
        );
      }
    }

    const result = await client.uploadFile(session, file, artefact);
    console.log(
      JSON.stringify(
        {
          uploadId: result.uploadId,
          file,
          artefact,
          plaintextBytes: result.bytes,
          envelopeBytes: result.envelopeBytes,
        },
        null,
        2,
      ),
    );
    return;
  }

  usage();
}

main().catch((err) => {
  console.error(err instanceof Error ? err.message : err);
  process.exit(1);
});
