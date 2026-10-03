import type { DownloadRequest } from "@vfdm/protocol";
import { findPort, ping, sendDownload } from "./bridge";
import { fromDownloadItem, fromLink, fromMediaItem, fromPageForYtdlp } from "./capture";
import { skipReason } from "./filters";
import { loadSettings, onSettingsChanged, type ExtSettings } from "./settings";
import { addItem, classify, clearTab, getTab, headerMap, setPageUrl, sizeOf, toItem, updateBadge } from "./sniff";

const MENU_ID = "vfdm-download";
const HEARTBEAT = "vfdm-heartbeat";

let settings: ExtSettings | null = null;
const settingsReady = loadSettings().then((s) => (settings = s));
onSettingsChanged((s) => (settings = s));

async function getSettings(): Promise<ExtSettings> {
  return settings ?? (await settingsReady);
}

// --- app status (global badge color/title; per-tab badge text = media count) --

async function setStatusBadge(online: boolean, enabled: boolean) {
  await chrome.action.setBadgeBackgroundColor({ color: !enabled ? "#6b7280" : online ? "#0ea5e9" : "#dc2626" });
  await chrome.action.setTitle({
    title: !enabled ? "vfdm: capture disabled" : online ? "vfdm: connected" : "vfdm: app not running",
  });
}

async function heartbeat(): Promise<number | null> {
  const s = await getSettings();
  const port = await findPort(s.port);
  await setStatusBadge(port !== null, s.enabled);
  return port;
}

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: MENU_ID,
    title: "Download with vfdm",
    contexts: ["link", "image", "video", "audio"],
  });
  chrome.alarms.create(HEARTBEAT, { periodInMinutes: 0.5 });
  void heartbeat();
});
chrome.runtime.onStartup.addListener(() => {
  chrome.alarms.create(HEARTBEAT, { periodInMinutes: 0.5 });
  void heartbeat();
});
chrome.alarms.onAlarm.addListener((a) => {
  if (a.name === HEARTBEAT) void heartbeat();
});
chrome.storage.onChanged.addListener((_c, area) => {
  if (area === "local") void heartbeat();
});

// --- notifications -------------------------------------------------------

function notify(title: string, message: string) {
  if (settings && !settings.notify) return;
  chrome.notifications.create({ type: "basic", iconUrl: "icons/128.png", title, message });
}

// --- send path -----------------------------------------------------------

export type Delivered = { ok: true; id: number } | { ok: false; message: string };

async function deliver(req: DownloadRequest): Promise<Delivered> {
  const s = await getSettings();
  const port = await findPort(s.port);
  if (port === null) return { ok: false, message: "vfdm app is not running" };
  if (!s.token) return { ok: false, message: "No token set — open vfdm extension options" };
  const r = await sendDownload(port, s.token, req);
  if (r.ok) return { ok: true, id: r.id };
  const message =
    r.reason === "unauthorized"
      ? "Token rejected — copy it again from vfdm Settings"
      : r.reason === "offline"
        ? "vfdm app is not running"
        : `vfdm refused: ${r.detail ?? r.reason}`;
  return { ok: false, message };
}

// --- download interception ----------------------------------------------
// onDeterminingFilename fires once headers are known, so filename/mime/size
// are populated. We never suggest a name; we either leave the download alone
// or cancel it and hand the URL (+cookies) to the app.

chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
  void (async () => {
    const s = await getSettings();
    if (!s.enabled || skipReason(s, item) !== null) {
      suggest();
      return;
    }
    const port = await findPort(s.port);
    if (port === null && s.offlinePolicy === "browser") {
      suggest();
      return;
    }
    try {
      await chrome.downloads.cancel(item.id);
    } catch {
      /* already gone */
    }
    try {
      suggest();
    } catch {
      /* cancelled */
    }
    chrome.downloads.erase({ id: item.id }, () => void chrome.runtime.lastError);

    const req = await fromDownloadItem(item);
    const res = await deliver(req);
    if (res.ok) return;
    if (s.offlinePolicy === "browser") {
      notify("vfdm unavailable", `${res.message}. Falling back to the browser.`);
      chrome.downloads.download({ url: item.finalUrl || item.url, filename: req.filename });
    } else {
      notify("vfdm blocked a download", res.message);
    }
  })();
  return true;
});

// --- media sniffing ------------------------------------------------------
// Observe-only webRequest (allowed in MV3). The engine fetches from outside
// the browser, so its own requests never show up here.

chrome.webRequest.onHeadersReceived.addListener(
  (d): undefined => {
    if (d.tabId < 0 || (d.statusCode !== 200 && d.statusCode !== 206)) return undefined;
    if (!/^https?:/i.test(d.url)) return undefined;
    const h = headerMap(d.responseHeaders);
    const size = sizeOf(h, d.statusCode);
    const kind = classify(d.url, h["content-type"] ?? "", size);
    if (!kind) return undefined;
    void addItem(d.tabId, toItem(d.url, kind, h["content-type"] ?? "", size));
    return undefined;
  },
  { urls: ["<all_urls>"], types: ["media", "xmlhttprequest", "other", "object", "main_frame", "sub_frame"] },
  ["responseHeaders"],
);

chrome.webNavigation.onCommitted.addListener((d) => {
  if (d.frameId === 0) void clearTab(d.tabId, d.url);
});
chrome.tabs.onRemoved.addListener((id) => void clearTab(id));
chrome.tabs.onActivated.addListener((a) => void updateBadge(a.tabId));

// --- context menu --------------------------------------------------------

chrome.contextMenus.onClicked.addListener((info, tab) => {
  if (info.menuItemId !== MENU_ID) return;
  const url = info.linkUrl || info.srcUrl;
  if (!url) return;
  void (async () => {
    const req = await fromLink(url, tab?.url ?? info.pageUrl);
    const res = await deliver(req);
    if (!res.ok) notify("vfdm", res.message);
  })();
});

// --- messages from popup/options -----------------------------------------

type Msg =
  | { type: "heartbeat" }
  | { type: "list-media"; tabId: number }
  | { type: "download-item"; tabId: number; key: string }
  | { type: "download-page-ytdlp"; tabId: number }
  | { type: "clear-media"; tabId: number };

chrome.runtime.onMessage.addListener((msg: Msg, _sender, reply) => {
  void (async () => {
    switch (msg.type) {
      case "heartbeat": {
        const port = await heartbeat();
        const info = port !== null ? await ping(port) : null;
        reply({ port, info });
        return;
      }
      case "list-media": {
        const t = await getTab(msg.tabId);
        if (t && !t.pageUrl) {
          const tab = await chrome.tabs.get(msg.tabId).catch(() => null);
          if (tab?.url) await setPageUrl(msg.tabId, tab.url);
        }
        reply(await getTab(msg.tabId));
        return;
      }
      case "download-item": {
        const t = await getTab(msg.tabId);
        const item = [...(t?.items ?? []), ...(t?.segments ?? [])].find((i) => i.key === msg.key);
        if (!item) return reply({ ok: false, message: "Item no longer listed" });
        const tab = await chrome.tabs.get(msg.tabId).catch(() => null);
        reply(await deliver(await fromMediaItem(item, t?.pageUrl || tab?.url || "")));
        return;
      }
      case "download-page-ytdlp": {
        const tab = await chrome.tabs.get(msg.tabId).catch(() => null);
        if (!tab?.url || !/^https?:/i.test(tab.url)) return reply({ ok: false, message: "No page URL" });
        reply(await deliver(await fromPageForYtdlp(tab.url)));
        return;
      }
      case "clear-media": {
        const tab = await chrome.tabs.get(msg.tabId).catch(() => null);
        await clearTab(msg.tabId, tab?.url);
        reply({ ok: true });
        return;
      }
    }
  })();
  return true;
});
