import {
  ENDPOINTS,
  PORT_RANGE,
  PROTOCOL_VERSION,
  bridgeBase,
  type DownloadRequest,
  type DownloadResponse,
  type PingResponse,
} from "@vfdm/protocol";

const SESSION_KEY = "bridge";
const PING_TIMEOUT_MS = 600;

interface Cached {
  port: number;
  at: number;
}

async function fetchTimeout(url: string, init: RequestInit = {}, ms = PING_TIMEOUT_MS): Promise<Response> {
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), ms);
  try {
    return await fetch(url, { ...init, signal: ctl.signal });
  } finally {
    clearTimeout(t);
  }
}

export async function ping(port: number): Promise<PingResponse | null> {
  try {
    const r = await fetchTimeout(bridgeBase(port) + ENDPOINTS.ping);
    if (!r.ok) return null;
    const j = (await r.json()) as PingResponse;
    return j.app === "vfdm" && j.protocol === PROTOCOL_VERSION ? j : null;
  } catch {
    return null;
  }
}

/** Returns the live port, scanning the range if the cached one went away. */
export async function findPort(preferred: number): Promise<number | null> {
  if (preferred > 0) return (await ping(preferred)) ? preferred : null;

  const cached = (await chrome.storage.session.get(SESSION_KEY))[SESSION_KEY] as Cached | undefined;
  if (cached && (await ping(cached.port))) return cached.port;

  for (let p = PORT_RANGE.start; p <= PORT_RANGE.end; p++) {
    if (await ping(p)) {
      await chrome.storage.session.set({ [SESSION_KEY]: { port: p, at: Date.now() } satisfies Cached });
      return p;
    }
  }
  await chrome.storage.session.remove(SESSION_KEY);
  return null;
}

export type SendResult =
  | { ok: true; id: number }
  | { ok: false; reason: "offline" | "unauthorized" | "rejected" | "error"; detail?: string };

export async function sendDownload(port: number, token: string, req: DownloadRequest, dry = false): Promise<SendResult> {
  try {
    const r = await fetchTimeout(
      bridgeBase(port) + ENDPOINTS.download + (dry ? "?dry=1" : ""),
      {
        method: "POST",
        headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
        body: JSON.stringify(req),
      },
      5000,
    );
    if (r.status === 401) return { ok: false, reason: "unauthorized" };
    if (r.status === 204) return { ok: true, id: 0 };
    if (!r.ok) return { ok: false, reason: "rejected", detail: await r.text() };
    const j = (await r.json()) as DownloadResponse;
    return { ok: true, id: j.id };
  } catch (e) {
    return { ok: false, reason: "offline", detail: String(e) };
  }
}
