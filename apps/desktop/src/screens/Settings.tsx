import { useEffect, useState } from "react";
import { api, type AppSettings, type BridgeInfo } from "../lib/ipc";

interface Props {
  open: boolean;
  onClose: () => void;
  onSaved: (s: AppSettings) => void;
}

export default function Settings({ open, onClose, onSaved }: Props) {
  const [s, setS] = useState<AppSettings | null>(null);
  const [bridge, setBridge] = useState<BridgeInfo | null>(null);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    api.getSettings().then(setS);
    api.getBridgeInfo().then(setBridge);
    setError(null);
  }, [open]);

  if (!open || !s) return null;

  const save = async () => {
    try {
      const saved = await api.setSettings(s);
      onSaved(saved);
      onClose();
    } catch (e) {
      setError(String(e));
    }
  };

  const copyToken = async () => {
    if (!bridge) return;
    await navigator.clipboard.writeText(bridge.token);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const field = "mt-1 w-full rounded-md border border-neutral-700 bg-neutral-950 px-3 py-1.5 text-sm outline-none focus:border-sky-600";

  return (
    <div className="fixed inset-0 z-20 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        className="w-[600px] max-w-[92vw] rounded-xl border border-neutral-800 bg-neutral-900 p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="text-base font-semibold">Settings</h2>

        <label className="mt-4 block text-xs text-neutral-400">Download folder</label>
        <div className="mt-1 flex gap-2">
          <input value={s.download_dir} onChange={(e) => setS({ ...s, download_dir: e.target.value })} className={`${field} mt-0 flex-1`} />
          <button
            onClick={async () => {
              const p = await api.pickDownloadDir();
              if (p) setS({ ...s, download_dir: p });
            }}
            className="rounded-md border border-neutral-700 px-3 text-sm hover:bg-neutral-800"
          >
            Browse
          </button>
        </div>

        <div className="mt-3 grid grid-cols-2 gap-3">
          <div>
            <label className="block text-xs text-neutral-400">Connections per download (1–16)</label>
            <input
              type="number"
              min={1}
              max={16}
              value={s.max_connections}
              onChange={(e) => setS({ ...s, max_connections: Number(e.target.value) || 1 })}
              className={field}
            />
          </div>
          <div>
            <label className="block text-xs text-neutral-400">Concurrent downloads (1–10)</label>
            <input
              type="number"
              min={1}
              max={10}
              value={s.max_concurrent}
              onChange={(e) => setS({ ...s, max_concurrent: Number(e.target.value) || 1 })}
              className={field}
            />
          </div>
        </div>

        <label className="mt-3 flex items-center gap-2 text-sm">
          <input type="checkbox" checked={s.notify_on_complete} onChange={(e) => setS({ ...s, notify_on_complete: e.target.checked })} />
          Notify when a download completes
        </label>
        <label className="mt-2 flex items-center gap-2 text-sm">
          <input type="checkbox" checked={s.focus_on_capture} onChange={(e) => setS({ ...s, focus_on_capture: e.target.checked })} />
          Bring window to front when the extension sends a download
        </label>

        <div className="mt-5 rounded-lg border border-neutral-800 bg-neutral-950/60 p-3">
          <div className="text-sm font-medium">Browser extension pairing</div>
          <p className="mt-1 text-xs text-neutral-400">
            Paste this token into the vfdm extension options page. The app listens on{" "}
            <span className="font-mono">127.0.0.1:{bridge?.port || "…"}</span>.
          </p>
          <div className="mt-2 flex items-center gap-2">
            <code className="flex-1 truncate rounded bg-neutral-900 px-2 py-1 font-mono text-xs text-neutral-300 select-text">
              {bridge?.token ?? "…"}
            </code>
            <button onClick={copyToken} className="rounded-md border border-neutral-700 px-3 py-1 text-xs hover:bg-neutral-800">
              {copied ? "Copied" : "Copy"}
            </button>
            <button
              onClick={async () => setBridge(await api.regenerateToken())}
              className="rounded-md border border-neutral-700 px-3 py-1 text-xs text-amber-300 hover:bg-neutral-800"
              title="Old token stops working immediately"
            >
              Regenerate
            </button>
          </div>
        </div>

        {error && <div className="mt-3 text-xs text-rose-300">{error}</div>}
        <div className="mt-5 flex justify-end gap-2">
          <button onClick={onClose} className="rounded-md px-3 py-1.5 text-sm text-neutral-300 hover:bg-neutral-800">
            Cancel
          </button>
          <button onClick={save} className="rounded-md bg-sky-600 px-4 py-1.5 text-sm font-medium hover:bg-sky-500">
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
