import { useEffect, useState } from "react";
import { api } from "../lib/ipc";

interface Props {
  open: boolean;
  onClose: () => void;
  defaultDir: string;
  defaultConnections: number;
}

export default function AddUrlDialog({ open, onClose, defaultDir, defaultConnections }: Props) {
  const [urls, setUrls] = useState("");
  const [filename, setFilename] = useState("");
  const [dir, setDir] = useState(defaultDir);
  const [conns, setConns] = useState(defaultConnections);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setDir(defaultDir);
    setConns(defaultConnections);
    setError(null);
    navigator.clipboard
      ?.readText()
      .then((t) => {
        if (/^https?:\/\/\S+$/i.test(t.trim()) && !urls) setUrls(t.trim());
      })
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  if (!open) return null;

  const submit = async () => {
    const list = urls
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean);
    if (list.length === 0) return setError("Paste at least one URL");
    try {
      for (const url of list) {
        await api.add({
          url,
          filename: list.length === 1 && filename ? filename : undefined,
          dest_dir: dir || undefined,
          max_connections: conns,
        });
      }
      setUrls("");
      setFilename("");
      onClose();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="fixed inset-0 z-20 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        className="w-[560px] max-w-[92vw] rounded-xl border border-neutral-800 bg-neutral-900 p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="text-base font-semibold">Add download</h2>
        <label className="mt-4 block text-xs text-neutral-400">URLs (one per line)</label>
        <textarea
          autoFocus
          value={urls}
          onChange={(e) => setUrls(e.target.value)}
          rows={4}
          spellCheck={false}
          className="mt-1 w-full rounded-md border border-neutral-700 bg-neutral-950 px-3 py-2 font-mono text-sm outline-none focus:border-sky-600"
          placeholder="https://example.com/file.zip"
        />
        <div className="mt-3 grid grid-cols-[1fr_120px] gap-3">
          <div>
            <label className="block text-xs text-neutral-400">Save as (optional, single URL)</label>
            <input
              value={filename}
              onChange={(e) => setFilename(e.target.value)}
              className="mt-1 w-full rounded-md border border-neutral-700 bg-neutral-950 px-3 py-1.5 text-sm outline-none focus:border-sky-600"
            />
          </div>
          <div>
            <label className="block text-xs text-neutral-400">Connections</label>
            <input
              type="number"
              min={1}
              max={16}
              value={conns}
              onChange={(e) => setConns(Math.min(16, Math.max(1, Number(e.target.value) || 1)))}
              className="mt-1 w-full rounded-md border border-neutral-700 bg-neutral-950 px-3 py-1.5 text-sm outline-none focus:border-sky-600"
            />
          </div>
        </div>
        <label className="mt-3 block text-xs text-neutral-400">Folder</label>
        <div className="mt-1 flex gap-2">
          <input
            value={dir}
            onChange={(e) => setDir(e.target.value)}
            className="flex-1 rounded-md border border-neutral-700 bg-neutral-950 px-3 py-1.5 text-sm outline-none focus:border-sky-600"
          />
          <button
            onClick={async () => {
              const picked = await api.pickDownloadDir();
              if (picked) setDir(picked);
            }}
            className="rounded-md border border-neutral-700 px-3 text-sm hover:bg-neutral-800"
          >
            Browse
          </button>
        </div>
        {error && <div className="mt-3 text-xs text-rose-300">{error}</div>}
        <div className="mt-5 flex justify-end gap-2">
          <button onClick={onClose} className="rounded-md px-3 py-1.5 text-sm text-neutral-300 hover:bg-neutral-800">
            Cancel
          </button>
          <button onClick={submit} className="rounded-md bg-sky-600 px-4 py-1.5 text-sm font-medium hover:bg-sky-500">
            Download
          </button>
        </div>
      </div>
    </div>
  );
}
