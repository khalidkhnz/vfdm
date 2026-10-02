import type { DownloadRequest } from "@vfdm/protocol";

/** Cookie header for a URL, as the browser would send it. */
export async function cookieHeader(...urls: string[]): Promise<string | undefined> {
  const seen = new Map<string, string>();
  for (const url of urls) {
    if (!url || !/^https?:/i.test(url)) continue;
    let list: chrome.cookies.Cookie[] = [];
    try {
      list = await chrome.cookies.getAll({ url });
    } catch {
      continue;
    }
    for (const c of list) {
      if (!seen.has(c.name)) seen.set(c.name, c.value);
    }
  }
  if (seen.size === 0) return undefined;
  return [...seen].map(([k, v]) => `${k}=${v}`).join("; ");
}

function basename(p: string | undefined): string | undefined {
  if (!p) return undefined;
  const b = p.split(/[\\/]/).pop();
  return b || undefined;
}

export async function fromDownloadItem(item: chrome.downloads.DownloadItem): Promise<DownloadRequest> {
  const url = item.finalUrl || item.url;
  return {
    url,
    original_url: item.url !== url ? item.url : undefined,
    filename: basename(item.filename),
    referrer: item.referrer || undefined,
    user_agent: navigator.userAgent,
    cookies: await cookieHeader(url, item.url),
  };
}

export async function fromLink(url: string, referrer?: string): Promise<DownloadRequest> {
  return {
    url,
    referrer: referrer || undefined,
    user_agent: navigator.userAgent,
    cookies: await cookieHeader(url, referrer ?? ""),
  };
}
