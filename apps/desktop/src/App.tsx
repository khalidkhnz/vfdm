import { useEffect, useState } from "react";
import { api, type AppSettings } from "./lib/ipc";
import { subscribeDownloads, sortedDownloads, useDownloads } from "./store/downloads";
import DownloadList from "./screens/DownloadList";
import Settings from "./screens/Settings";
import AddUrlDialog from "./components/AddUrlDialog";
import BridgeStatus from "./components/BridgeStatus";

export default function App() {
  const [addOpen, setAddOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const items = useDownloads((s) => s.items);

  useEffect(() => {
    const unsub = subscribeDownloads();
    api.getSettings().then(setSettings);
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "n") {
        e.preventDefault();
        setAddOpen(true);
      }
      if (e.key === "Escape") {
        setAddOpen(false);
        setSettingsOpen(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      unsub();
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  const all = sortedDownloads(items);
  const active = all.filter((p) => p.status.state === "downloading" || p.status.state === "probing");
  const paused = all.filter((p) => p.status.state === "paused");
  const totalSpeed = active.reduce((a, p) => a + p.speed_bps, 0);

  return (
    <div className="flex h-screen flex-col">
      <header className="flex items-center gap-2 border-b border-neutral-800 px-4 py-2.5">
        <span className="mr-2 text-sm font-semibold tracking-tight">vfdm</span>
        <button
          onClick={() => setAddOpen(true)}
          className="rounded-md bg-sky-600 px-3 py-1 text-xs font-medium hover:bg-sky-500"
          title="⌘N"
        >
          + Add URL
        </button>
        <button
          onClick={() => active.forEach((p) => api.pause(p.id))}
          disabled={active.length === 0}
          className="rounded-md px-3 py-1 text-xs text-neutral-300 hover:bg-neutral-800 disabled:opacity-40"
        >
          Pause all
        </button>
        <button
          onClick={() => paused.forEach((p) => api.resume(p.id))}
          disabled={paused.length === 0}
          className="rounded-md px-3 py-1 text-xs text-neutral-300 hover:bg-neutral-800 disabled:opacity-40"
        >
          Resume all
        </button>
        <div className="flex-1" />
        {totalSpeed > 0 && (
          <span className="text-xs text-neutral-400 tabular-nums">{(totalSpeed / 1048576).toFixed(1)} MB/s</span>
        )}
        <BridgeStatus onClick={() => setSettingsOpen(true)} />
        <button
          onClick={() => setSettingsOpen(true)}
          className="rounded-md px-2 py-1 text-xs text-neutral-300 hover:bg-neutral-800"
          title="Settings"
        >
          ⚙
        </button>
      </header>

      <DownloadList />

      <AddUrlDialog
        open={addOpen}
        onClose={() => setAddOpen(false)}
        defaultDir={settings?.download_dir ?? ""}
        defaultConnections={settings?.max_connections ?? 8}
      />
      <Settings open={settingsOpen} onClose={() => setSettingsOpen(false)} onSaved={setSettings} />
    </div>
  );
}
