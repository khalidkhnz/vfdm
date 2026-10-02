import type { ExtSettings } from "./settings";

export function extensionOf(filename: string | undefined, url: string): string {
  const name = filename || url.split(/[?#]/)[0].split("/").pop() || "";
  const i = name.lastIndexOf(".");
  return i >= 0 ? name.slice(i + 1).toLowerCase() : "";
}

export function hostOf(url: string): string {
  try {
    return new URL(url).hostname.toLowerCase();
  } catch {
    return "";
  }
}

/** Returns a reason string when the item should be left to the browser, else null. */
export function skipReason(s: ExtSettings, item: chrome.downloads.DownloadItem): string | null {
  const url = item.finalUrl || item.url;
  if (!/^https?:/i.test(url)) return "non-http";
  if (item.byExtensionId === chrome.runtime.id) return "own-fallback";

  const host = hostOf(url);
  if (s.skipHosts.some((h) => host === h || host.endsWith("." + h))) return "host-skipped";

  if (s.extensions.length > 0) {
    const ext = extensionOf(item.filename, url);
    if (!s.extensions.includes(ext)) return `ext:${ext || "none"}`;
  }

  if (s.minSizeMb > 0 && item.totalBytes > 0 && item.totalBytes < s.minSizeMb * 1024 * 1024) {
    return "too-small";
  }
  return null;
}
