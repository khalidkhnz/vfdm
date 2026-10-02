import { findPort, sendDownload } from "./bridge";
import { fromDownloadItem, fromLink } from "./capture";
import { skipReason } from "./filters";
import { loadSettings, onSettingsChanged, type ExtSettings } from "./settings";

const MENU_ID = "vfdm-download";
const HEARTBEAT = "vfdm-heartbeat";

let settings: ExtSettings | null = null;
const settingsReady = loadSettings().then((s) => (settings = s));
onSettingsChanged((s) => (settings = s));

async function getSettings(): Promise<ExtSettings> {
  return settings ?? (await settingsReady);
}

// --- badge / heartbeat ---------------------------------------------------

async function setBadge(online: boolean, enabled: boolean) {
  await chrome.action.setBadgeText({ text: !enabled ? "off" : online ? "" : "!" });
  await chrome.action.setBadgeBackgroundColor({ color: !enabled ? "#6b7280" : "#dc2626" });
  await chrome.action.setTitle({
    title: !enabled ? "vfdm: capture disabled" : online ? "vfdm: connected" : "vfdm: app not running",
  });
}

async function heartbeat(): Promise<number | null> {
  const s = await getSettings();
  const port = await findPort(s.port);
  await setBadge(port !== null, s.enabled);
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
chrome.storage.onChanged.addListener(() => void heartbeat());

// --- notifications -------------------------------------------------------

function notify(title: string, message: string) {
  if (settings && !settings.notify) return;
  chrome.notifications.create({ type: "basic", iconUrl: "icons/128.png", title, message });
}

// --- send path -----------------------------------------------------------

type Delivered = { ok: true } | { ok: false; message: string };

async function deliver(req: Parameters<typeof sendDownload>[2]): Promise<Delivered> {
  const s = await getSettings();
  const port = await findPort(s.port);
  if (port === null) return { ok: false, message: "vfdm app is not running" };
  if (!s.token) return { ok: false, message: "No token set — open vfdm extension options" };
  const r = await sendDownload(port, s.token, req);
  if (r.ok) return { ok: true };
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
// are populated. We never call suggest() with a name; we either leave the
// download alone or cancel it and hand the URL (+cookies) to the app.

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

chrome.runtime.onMessage.addListener((msg: { type: string }, _sender, reply) => {
  if (msg.type === "heartbeat") {
    heartbeat().then((port) => reply({ port }));
    return true;
  }
  return false;
});
