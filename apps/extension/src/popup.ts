import type { PingResponse } from "@vfdm/protocol";
import { loadSettings, saveSettings } from "./settings";
import type { MediaItem, TabMedia } from "./sniff";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const msg = $("msg");

function send<T>(m: unknown): Promise<T> {
  return chrome.runtime.sendMessage(m) as Promise<T>;
}

function bytes(n: number | null): string {
  if (n === null) return "";
  const u = ["B", "KB", "MB", "GB"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${u[i]}`;
}

function show(text: string, cls: "ok" | "err") {
  msg.textContent = text;
  msg.className = `msg ${cls}`;
}

function row(item: MediaItem, tabId: number): HTMLLIElement {
  const li = document.createElement("li");
  const tag = document.createElement("span");
  tag.className = `tag ${item.kind}`;
  tag.textContent = { hls: "HLS", dash: "DASH", file: item.mime.startsWith("audio/") ? "AUDIO" : "VIDEO", segment: "CHUNK" }[item.kind];
  const mid = document.createElement("div");
  const name = document.createElement("div");
  name.className = "name";
  name.textContent = item.filename;
  name.title = item.url;
  const meta = document.createElement("div");
  meta.className = "meta";
  meta.textContent = [item.host, bytes(item.size)].filter(Boolean).join(" · ");
  mid.append(name, meta);
  const btn = document.createElement("button");
  btn.textContent = "Download";
  btn.addEventListener("click", async () => {
    btn.disabled = true;
    const r = await send<{ ok: boolean; message?: string }>({ type: "download-item", tabId, key: item.key });
    if (r.ok) show(`Sent ${item.filename} to vfdm`, "ok");
    else show(r.message ?? "Failed", "err");
    btn.disabled = false;
  });
  li.append(tag, mid, btn);
  return li;
}

async function main() {
  const s = await loadSettings();
  const enabled = $<HTMLInputElement>("enabled");
  enabled.checked = s.enabled;
  enabled.addEventListener("change", async () => {
    await saveSettings({ ...(await loadSettings()), enabled: enabled.checked });
  });
  $("options").addEventListener("click", () => chrome.runtime.openOptionsPage());

  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const tabId = tab?.id ?? -1;

  const status = await send<{ port: number | null; info: PingResponse | null }>({ type: "heartbeat" });
  const dot = $("dot");
  const text = $("text");
  if (status.port) {
    dot.className = "dot on";
    text.textContent = `Connected (port ${status.port})`;
  } else {
    dot.className = "dot off";
    text.textContent = "vfdm app not running";
  }
  const ytBtn = $<HTMLButtonElement>("ytdlp");
  const ytOk = !!status.info?.tools?.ytdlp;
  ytBtn.disabled = !status.port || !ytOk || !tab?.url || !/^https?:/i.test(tab.url);
  if (status.port && !ytOk) ytBtn.title = "Install yt-dlp in vfdm → Settings → Tools";
  ytBtn.addEventListener("click", async () => {
    ytBtn.disabled = true;
    const r = await send<{ ok: boolean; message?: string }>({ type: "download-page-ytdlp", tabId });
    show(r.ok ? "Page sent to yt-dlp" : (r.message ?? "Failed"), r.ok ? "ok" : "err");
    ytBtn.disabled = false;
  });

  const media = tabId >= 0 ? await send<TabMedia | null>({ type: "list-media", tabId }) : null;
  const items = media?.items ?? [];
  const segs = media?.segments ?? [];
  const ul = $("items");
  for (const it of items) ul.append(row(it, tabId));
  $("empty").hidden = items.length > 0;
  if (segs.length > 0 && items.length === 0) {
    $("segwrap").hidden = false;
    $("segsum").textContent = `${segs.length} stream chunks seen (no playlist) — prefer yt-dlp`;
    const sl = $("segments");
    for (const it of segs) sl.append(row(it, tabId));
  }
}

void main();
