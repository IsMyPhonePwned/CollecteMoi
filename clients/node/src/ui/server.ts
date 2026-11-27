#!/usr/bin/env node
/**
 * Local desktop client UI for CollecteMoi.
 *
 * Serves a small browser UI and uses the same enroll / lab-seal / upload
 * client as the CLI. Flags only (no env config).
 */

import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { readFile, unlink, mkdtemp, writeFile, rm } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { isIP } from "node:net";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";

import { CollecteMoiClient, type Artefact } from "../client.js";
import { loadSession, saveSession, type Session } from "../session.js";
import { configureTls } from "../tls.js";

type Opts = {
  bind: string;
  port: number;
  apiServer: string;
  sessionPath: string;
  open: boolean;
  insecure: boolean;
  tlsCa?: string;
};

function parseArgs(argv: string[]): Opts {
  const opts: Opts = {
    bind: "127.0.0.1",
    port: 8090,
    apiServer: "https://127.0.0.1:8080",
    sessionPath: resolve("./.collectemoi/session.json"),
    open: false,
    insecure: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]!;
    const next = argv[i + 1];
    switch (a) {
      case "--bind":
        opts.bind = next ?? opts.bind;
        i++;
        break;
      case "--port":
        opts.port = Number(next ?? opts.port);
        i++;
        break;
      case "--server":
        opts.apiServer = (next ?? opts.apiServer).replace(/\/$/, "");
        i++;
        break;
      case "--session":
        opts.sessionPath = resolve(next ?? opts.sessionPath);
        i++;
        break;
      case "--no-open":
        opts.open = false;
        break;
      case "--open":
        opts.open = true;
        break;
      case "--insecure":
        opts.insecure = true;
        break;
      case "--secure":
        opts.insecure = false;
        break;
      case "--tls-ca":
        opts.tlsCa = next;
        opts.insecure = false;
        i++;
        break;
      case "-h":
      case "--help":
        console.log(`Usage: collectemoi-client-ui [options]

Options:
  --bind <host>       Listen host (default: 127.0.0.1)
  --port <n>          Listen port (default: 8090)
  --server <url>      CollecteMoi API base URL (default: https://127.0.0.1:8080)
  --session <path>    Session JSON path (default: ./.collectemoi/session.json)
  --insecure          Skip API certificate verification (lab only; off by default)
  --secure            Require a valid TLS certificate
  --tls-ca <pem>      Trust this CA/cert PEM
  --no-open           Do not open a browser tab (default)
  --open              Open a browser tab on start
`);
        process.exit(0);
        break;
      default:
        if (a.startsWith("-")) {
          console.error(`unknown option: ${a}`);
          process.exit(1);
        }
    }
  }
  return opts;
}

function sendJson(res: ServerResponse, status: number, body: unknown) {
  const data = JSON.stringify(body);
  res.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
    "x-content-type-options": "nosniff",
    "referrer-policy": "no-referrer",
    "x-frame-options": "DENY",
  });
  res.end(data);
}

function isLocalUiRequest(req: IncomingMessage, port: number): boolean {
  const rawHost = req.headers.host;
  if (!rawHost) return false;
  let host: URL;
  try {
    host = new URL(`http://${rawHost}`);
  } catch {
    return false;
  }
  const hostname = host.hostname.replace(/^\[|\]$/g, "").toLowerCase();
  if (hostname !== "localhost" && isIP(hostname) === 0) return false;
  const portText = host.port || "80";
  if (Number(portText) !== port) return false;

  const origin = req.headers.origin;
  if (!origin) return true;
  try {
    const parsedOrigin = new URL(origin);
    return parsedOrigin.origin === host.origin && parsedOrigin.protocol === "http:";
  } catch {
    return false;
  }
}

async function readBody(req: IncomingMessage): Promise<Buffer> {
  const chunks: Buffer[] = [];
  for await (const chunk of req) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  }
  return Buffer.concat(chunks);
}

async function maybeOpen(url: string) {
  const { spawn } = await import("node:child_process");
  const platform = process.platform;
  if (platform === "darwin") spawn("open", [url], { stdio: "ignore", detached: true });
  else if (platform === "win32") spawn("cmd", ["/c", "start", url], { stdio: "ignore", detached: true });
  else spawn("xdg-open", [url], { stdio: "ignore", detached: true });
}

async function publicSession(path: string): Promise<Record<string, unknown> | null> {
  try {
    const s = await loadSession(path);
    return {
      deviceId: s.deviceId,
      caseId: s.caseId,
      displayName: s.displayName,
      serverBaseUrl: s.serverBaseUrl,
      enrolledAt: s.enrolledAt,
      enabledCollectors: s.enabledCollectors,
      email: s.email,
      companyId: s.companyId,
      noiseKeyFingerprint: s.noiseKeyFingerprint,
      devicePublicKeyHex: s.devicePublicKeyHex,
      casePublicKeyHex: s.casePublicKeyHex,
    };
  } catch {
    return null;
  }
}

async function requireSession(path: string): Promise<Session> {
  try {
    return await loadSession(path);
  } catch {
    throw new Error("Not enrolled — use Enroll first");
  }
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  configureTls({ insecure: opts.insecure, tlsCa: opts.tlsCa });
  const here = dirname(fileURLToPath(import.meta.url));
  const pageHtml = await readFile(resolve(here, "page.html"), "utf8");

  const server = createServer(async (req, res) => {
    try {
      if (!isLocalUiRequest(req, opts.port)) {
        sendJson(res, 403, { error: "local client UI request rejected" });
        return;
      }
      const url = new URL(req.url ?? "/", `http://${opts.bind}:${opts.port}`);
      const path = url.pathname;

      if (req.method === "GET" && (path === "/" || path === "/index.html")) {
        res.writeHead(200, {
          "content-type": "text/html; charset=utf-8",
          "cache-control": "no-store",
          "x-content-type-options": "nosniff",
          "referrer-policy": "no-referrer",
          "x-frame-options": "DENY",
          "content-security-policy": "frame-ancestors 'none'",
        });
        res.end(pageHtml);
        return;
      }

      if (req.method === "GET" && path === "/api/session") {
        sendJson(res, 200, {
          session: await publicSession(opts.sessionPath),
          apiServer: opts.apiServer,
        });
        return;
      }

      if (req.method === "DELETE" && path === "/api/session") {
        try {
          await unlink(opts.sessionPath);
        } catch (err) {
          const code = (err as NodeJS.ErrnoException).code;
          if (code !== "ENOENT") throw err;
        }
        sendJson(res, 200, { cleared: true });
        return;
      }

      if (req.method === "GET" && path === "/api/health") {
        try {
          const client = new CollecteMoiClient(opts.apiServer);
          const text = await client.healthz();
          sendJson(res, 200, {
            ok: text.trim().toLowerCase() === "ok" || text.trim().length > 0,
            apiServer: opts.apiServer,
            healthz: text.trim(),
          });
        } catch (err) {
          const msg = err instanceof Error ? err.message : String(err);
          sendJson(res, 200, {
            ok: false,
            apiServer: opts.apiServer,
            error: msg,
          });
        }
        return;
      }

      if (req.method === "POST" && path === "/api/enroll/start") {
        const raw = await readBody(req);
        const body = JSON.parse(raw.toString("utf8") || "{}") as {
          server?: string;
          email?: string;
          lab_key?: string;
        };
        const api = (body.server || opts.apiServer).replace(/\/$/, "");
        const client = new CollecteMoiClient(api);
        const started = await client.enrollStart({
          email: body.email,
          labKey: body.lab_key,
        });
        sendJson(res, 200, started);
        return;
      }

      if (req.method === "POST" && path === "/api/enroll/confirm") {
        const raw = await readBody(req);
        const body = JSON.parse(raw.toString("utf8") || "{}") as {
          server?: string;
          challengeId?: string;
          code?: string;
          email?: string;
          companyId?: string;
        };
        const api = (body.server || opts.apiServer).replace(/\/$/, "");
        if (!body.challengeId || !body.code) {
          sendJson(res, 400, { error: "challengeId and code required" });
          return;
        }
        const client = new CollecteMoiClient(api);
        const session = await client.enrollConfirm({
          challengeId: body.challengeId,
          code: body.code,
          email: body.email,
          companyId: body.companyId,
        });
        await saveSession(opts.sessionPath, session);
        sendJson(res, 201, {
          session: await publicSession(opts.sessionPath),
        });
        return;
      }

      if (req.method === "POST" && path === "/api/enroll") {
        // Convenience: email/lab_key (+ optional code) or legacy token.
        const raw = await readBody(req);
        const body = JSON.parse(raw.toString("utf8") || "{}") as {
          server?: string;
          token?: string;
          email?: string;
          lab_key?: string;
          code?: string;
        };
        const api = (body.server || opts.apiServer).replace(/\/$/, "");
        const client = new CollecteMoiClient(api);
        const session = body.token
          ? await client.enroll({ token: body.token.trim() })
          : await client.enrollWithEmail({
              email: body.email,
              labKey: body.lab_key,
              code: body.code,
            });
        await saveSession(opts.sessionPath, session);
        sendJson(res, 201, {
          session: await publicSession(opts.sessionPath),
        });
        return;
      }

      if (req.method === "GET" && path === "/api/requests") {
        const session = await requireSession(opts.sessionPath);
        const client = new CollecteMoiClient(session.serverBaseUrl);
        const requests = await client.requests();
        sendJson(res, 200, { requests });
        return;
      }

      if (req.method === "GET" && path === "/api/submissions") {
        const session = await requireSession(opts.sessionPath);
        const client = new CollecteMoiClient(session.serverBaseUrl);
        const submissions = await client.submissions(session);
        sendJson(res, 200, { submissions });
        return;
      }

      if (req.method === "POST" && path === "/api/upload") {
        const session = await requireSession(opts.sessionPath);
        const artefact = (url.searchParams.get("artefact") ||
          "android_bugreport") as Artefact;
        const filename =
          url.searchParams.get("filename") || "upload.bin";
        const bytes = await readBody(req);
        if (!bytes.length) {
          sendJson(res, 400, { error: "empty body" });
          return;
        }

        const dir = await mkdtemp(resolve(tmpdir(), "collectemoi-ui-"));
        const filePath = resolve(dir, filename.replace(/[^\w.-]+/g, "_"));
        try {
          await writeFile(filePath, bytes);
          const client = new CollecteMoiClient(session.serverBaseUrl);
          const result = await client.uploadFile(session, filePath, artefact);
          sendJson(res, 200, {
            uploadId: result.uploadId,
            filename,
            artefact,
            plaintextBytes: result.bytes,
            envelopeBytes: result.envelopeBytes,
          });
        } finally {
          await rm(dir, { recursive: true, force: true });
        }
        return;
      }

      sendJson(res, 404, { error: "not found" });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      const cause =
        err instanceof Error && err.cause instanceof Error
          ? err.cause.message
          : err instanceof Error && err.cause
            ? String(err.cause)
            : "";
      const detail = cause ? `${msg}: ${cause}` : msg;
      const unreachable =
        /fetch failed|ECONNREFUSED|ENOTFOUND|ECONNRESET|socket hang up/i.test(
          detail,
        );
      sendJson(res, 500, {
        error: unreachable
          ? `API unreachable at ${opts.apiServer} (${detail}). Is collectemoictl running?`
          : detail,
      });
    }
  });

  server.listen(opts.port, opts.bind, () => {
    const url = `http://${opts.bind}:${opts.port}/`;
    console.log(`CollecteMoi client UI → ${url}`);
    console.log(`API server           → ${opts.apiServer}`);
    console.log(`Session              → ${opts.sessionPath}`);
    if (!new Set(["127.0.0.1", "::1", "localhost"]).has(opts.bind)) {
      console.warn("Warning: the client UI is bound beyond loopback; expose it only on a trusted network.");
    }
    if (opts.open) void maybeOpen(url);
  });
}

main().catch((err) => {
  console.error(err instanceof Error ? err.message : err);
  process.exit(1);
});
