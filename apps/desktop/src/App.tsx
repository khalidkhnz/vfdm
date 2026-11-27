import { useCallback, useEffect, useState } from "react";
import { api, events, IN_TAURI, type AppSettings } from "./lib/ipc";
import { subscribeDownloads, useDownloads } from "./store/downloads";
import DownloadList from "./screens/DownloadList";
import Settings from "./screens/Settings";
import AddUrlDialog from "./components/AddUrlDialog";
import Rail from "./components/Rail";
import Throughput from "./components/Throughput";
import Toasts, { useToasts } from "./components/Toast";

const IS_MAC = /Mac/i.test(navigator.platform) || /Macintosh/i.test(navigator.userAgent);

function useMediaQuery(q: string): boolean {
  const [m, setM] = useState(() => window.matchMedia(q).matches);
  useEffect(() => {
    const mq = window.matchMedia(q);
    const on = () => setM(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, [q]);
  return m;
}

function linksFrom(dt: DataTransfer): string[] {
  const raw = dt.getData("text/uri-list") || dt.getData("text/plain") || "";
  return raw
    .split(/\r?\n/)
    .map((s) => s.trim())
    .filter((s) => /^https?:\/\//i.test(s));
}

export default function App() {
  const [addOpen, setAddOpen] = useState(false);
  const [addPrefill, setAddPrefill] = useState<string | undefined>();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [dragging, setDragging] = useState(false);
  const compact = useMediaQuery("(max-width: 760px)");
  const push = useToasts((s) => s.push);
  const setFilter = useDownloads((s) => s.setFilter);

  const openAdd = useCallback((prefill?: string) => {
    setAddPrefill(prefill);
    setAddOpen(true);
  }, []);

  useEffect(() => {
    const unsub = subscribeDownloads();
    api.getSettings().then(setSettings);
    const unBridge = events.onBridgeRequest((p) => push({ kind: "info", title: "Captured from the browser", body: p.filename || p.url }));

    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "n") {
        e.preventDefault();
        openAdd();
      } else if (mod && e.key === ",") {
        e.preventDefault();
        setSettingsOpen(true);
      } else if (mod && e.key >= "1" && e.key <= "4") {
        e.preventDefault();
        setFilter((["all", "active", "done", "failed"] as const)[Number(e.key) - 1]);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      unsub();
      window.removeEventListener("keydown", onKey);
      void unBridge.then((f) => f());
    };
  }, [openAdd, push, setFilter]);

  const onDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    setDragging(false);
    const links = linksFrom(e.dataTransfer);
    if (links.length === 0) return;
    if (links.length === 1) return openAdd(links[0]);
    for (const url of links) await api.add({ url });
    push({ kind: "info", title: `Added ${links.length} downloads` });
  };

  return (
    <div
      className={`flex h-screen flex-col bg-bg text-ink ${dragging ? "drop-ring" : ""}`}
      onDragOver={(e) => {
        e.preventDefault();
        if (!dragging) setDragging(true);
      }}
      onDragLeave={(e) => {
        if (e.currentTarget === e.target) setDragging(false);
      }}
      onDrop={onDrop}
    >
      <header
        data-tauri-drag-region
        className={`flex shrink-0 items-end justify-between gap-6 border-b border-line pr-5 pb-3 ${
          IS_MAC && IN_TAURI ? "pt-7 pl-[84px]" : "pt-4 pl-5"
        }`}
      >
        <div data-tauri-drag-region className="flex items-end gap-6">
          <span data-tauri-drag-region className="pb-1.5 text-[15px] font-semibold tracking-tight">
            vfdm
          </span>
          <Throughput />
        </div>
        {dragging && <span className="pb-1.5 text-[13px] text-accent">Drop links to download</span>}
      </header>

      <div className="flex min-h-0 flex-1">
        <Rail onAdd={() => openAdd()} onSettings={() => setSettingsOpen(true)} compact={compact} />
        <main className="flex min-w-0 flex-1 flex-col">
          <DownloadList onAdd={() => openAdd()} />
        </main>
      </div>

      <AddUrlDialog
        open={addOpen}
        onClose={() => setAddOpen(false)}
        defaultDir={settings?.download_dir ?? ""}
        defaultConnections={settings?.max_connections ?? 8}
        initialUrls={addPrefill}
      />
      <Settings open={settingsOpen} onClose={() => setSettingsOpen(false)} onSaved={setSettings} />
      <Toasts />
    </div>
  );
}
