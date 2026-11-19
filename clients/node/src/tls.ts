/**
 * Lab TLS helpers for self-signed CollecteMoi API certs.
 * Use --insecure to skip verification, or --tls-ca <pem> to trust a specific CA/cert.
 */

import { readFileSync } from "node:fs";
import { Agent, setGlobalDispatcher } from "undici";

let configured = false;

export function configureTls(opts: {
  insecure?: boolean;
  tlsCa?: string;
}): void {
  if (configured) return;
  if (!opts.insecure && !opts.tlsCa) return;

  if (opts.insecure) {
    setGlobalDispatcher(
      new Agent({
        connect: { rejectUnauthorized: false },
      }),
    );
    configured = true;
    return;
  }

  if (opts.tlsCa) {
    const ca = readFileSync(opts.tlsCa);
    setGlobalDispatcher(
      new Agent({
        connect: { ca, rejectUnauthorized: true },
      }),
    );
    configured = true;
  }
}
