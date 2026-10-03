/**
 * Media sniffing: classify responses seen by webRequest and keep a per-tab
 * list in chrome.storage.session. No content scripts, no page access.
 */

export type MediaKind = "hls" | "dash" | "file" | "segment";

export interface MediaItem {
  /** origin + pathname: collapses per-range requests and refreshed tokens. */
  key: string;
  url: string;
  kind: MediaKind;
  mime: string;
  size: number | null;
  host: string;
  filename: string;
  at: number;
}

export interface TabMedia {
  pageUrl: string;
  items: MediaItem[];
  segments: MediaItem[];
}

const MAX_ITEMS = 50;
const MAX_SEGMENTS = 10;
const MIN_FILE_BYTES = 200 * 1024;

const MANIFEST_MIMES = new Set([
  "application/vnd.apple.mpegurl",
  "application/x-mpegurl",
  "application/mpegurl",
  "audio/mpegurl",
  "audio/x-mpegurl",
]);
const DASH_MIMES = new Set(["application/dash+xml"]);
const SEGMENT_EXT = new Set(["ts", "m4s", "m4f", "cmfv", "cmfa"]);
const MEDIA_EXT = new Set(["mp4", "webm", "mkv", "mov", "flv", "avi", "m4v", "mp3", "m4a", "aac", "flac", "ogg", "opus", "wav"]);

function pathOf(url: string): { origin: string; pathname: string; ext: string; name: string } {
  try {
    const u = new URL(url);
    const name = u.pathname.split("/").pop() || "";
    const dot = name.lastIndexOf(".");
    return {
      origin: u.origin,
      pathname: u.pathname,
      ext: dot >= 0 ? name.slice(dot + 1).toLowerCase() : "",
      name,
    };
  } catch {
    return { origin: "", pathname: url, ext: "", name: "" };
  }
}

export function classify(url: string, mime: string, size: number | null): MediaKind | null {
  const m = (mime.split(";")[0] || "").trim().toLowerCase();
  const { ext } = pathOf(url);
  if (MANIFEST_MIMES.has(m) || ext === "m3u8" || ext === "m3u") return "hls";
  if (DASH_MIMES.has(m) || ext === "mpd") return "dash";
  if (m === "video/mp2t" || SEGMENT_EXT.has(ext)) return "segment";
  if (m === "text/html" || m.startsWith("text/") || m === "application/json") return null;
  const mediaByMime = m.startsWith("video/") || m.startsWith("audio/");
  const mediaByExt = MEDIA_EXT.has(ext) && (m === "" || m === "application/octet-stream" || mediaByMime);
  if (!mediaByMime && !mediaByExt) return null;
  if (size !== null && size < MIN_FILE_BYTES) return null;
  return "file";
}

export function toItem(url: string, kind: MediaKind, mime: string, size: number | null): MediaItem {
  const { origin, pathname, name } = pathOf(url);
  let host = "";
  try {
    host = new URL(url).hostname;
  } catch {
    /* keep empty */
  }
  return {
    key: origin + pathname,
    url,
    kind,
    mime: (mime.split(";")[0] || "").trim().toLowerCase(),
    size,
    host,
    filename: decodeURIComponent(name || "") || host || "media",
    at: Date.now(),
  };
}

const KEY = (tabId: number) => `tab:${tabId}`;
const chains = new Map<number, Promise<void>>();

/** Serialize read-modify-write per tab; the service worker is single-threaded
 *  but storage calls interleave. */
function withTab(tabId: number, fn: (t: TabMedia) => TabMedia | null): Promise<void> {
  const prev = chains.get(tabId) ?? Promise.resolve();
  const next = prev
    .then(async () => {
      const cur = (await getTab(tabId)) ?? { pageUrl: "", items: [], segments: [] };
      const updated = fn(cur);
      if (updated === null) await chrome.storage.session.remove(KEY(tabId));
      else await chrome.storage.session.set({ [KEY(tabId)]: updated });
      await updateBadge(tabId, updated);
    })
    .catch(() => {});
  chains.set(tabId, next);
  return next;
}

export async function getTab(tabId: number): Promise<TabMedia | null> {
  const r = await chrome.storage.session.get(KEY(tabId));
  return (r[KEY(tabId)] as TabMedia | undefined) ?? null;
}

export function addItem(tabId: number, item: MediaItem): Promise<void> {
  return withTab(tabId, (t) => {
    const list = item.kind === "segment" ? t.segments : t.items;
    const existing = list.find((i) => i.key === item.key);
    if (existing) {
      existing.url = item.url;
      existing.size = item.size ?? existing.size;
      existing.at = item.at;
      return t;
    }
    list.unshift(item);
    if (item.kind === "hls" || item.kind === "dash") t.segments = [];
    t.items = t.items.slice(0, MAX_ITEMS);
    t.segments = t.segments.slice(0, MAX_SEGMENTS);
    return t;
  });
}

export function setPageUrl(tabId: number, pageUrl: string): Promise<void> {
  return withTab(tabId, (t) => ({ ...t, pageUrl }));
}

export function clearTab(tabId: number, pageUrl?: string): Promise<void> {
  return withTab(tabId, () => (pageUrl ? { pageUrl, items: [], segments: [] } : null));
}

export async function updateBadge(tabId: number, t?: TabMedia | null): Promise<void> {
  const tab = t === undefined ? await getTab(tabId) : t;
  const n = tab ? tab.items.length + (tab.items.length === 0 ? tab.segments.length : 0) : 0;
  try {
    await chrome.action.setBadgeText({ tabId, text: n > 0 ? String(n) : "" });
    await chrome.action.setBadgeBackgroundColor({ tabId, color: "#0ea5e9" });
  } catch {
    /* tab gone */
  }
}

export function headerMap(headers: chrome.webRequest.HttpHeader[] | undefined): Record<string, string> {
  const out: Record<string, string> = {};
  for (const h of headers ?? []) out[h.name.toLowerCase()] = h.value ?? "";
  return out;
}

/** Full size from Content-Length, or the total in Content-Range for 206. */
export function sizeOf(h: Record<string, string>, status: number): number | null {
  if (status === 206) {
    const m = /\/(\d+)\s*$/.exec(h["content-range"] ?? "");
    if (m) return Number(m[1]);
  }
  const cl = Number(h["content-length"]);
  return Number.isFinite(cl) && cl > 0 ? cl : null;
}
