import { useMemo, useState } from "react";
import { sortedDownloads, useDownloads } from "../store/downloads";
import DownloadRow from "../components/DownloadRow";
import type { Progress } from "../lib/ipc";

type Filter = "all" | "active" | "done" | "failed";

const FILTERS: { key: Filter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "active", label: "Active" },
  { key: "done", label: "Done" },
  { key: "failed", label: "Failed" },
];

function matches(p: Progress, f: Filter): boolean {
  const s = p.status.state;
  switch (f) {
    case "all":
      return true;
    case "active":
      return s === "queued" || s === "probing" || s === "downloading" || s === "paused";
    case "done":
      return s === "completed";
    case "failed":
      return s === "failed" || s === "cancelled";
  }
}

export default function DownloadList() {
  const items = useDownloads((s) => s.items);
  const loaded = useDownloads((s) => s.loaded);
  const [filter, setFilter] = useState<Filter>("all");
  const list = useMemo(() => sortedDownloads(items).filter((p) => matches(p, filter)), [items, filter]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex gap-1 border-b border-neutral-800 px-4 py-2">
        {FILTERS.map((f) => (
          <button
            key={f.key}
            onClick={() => setFilter(f.key)}
            className={`rounded-md px-3 py-1 text-xs ${
              filter === f.key ? "bg-neutral-800 text-neutral-100" : "text-neutral-400 hover:bg-neutral-900"
            }`}
          >
            {f.label}
          </button>
        ))}
      </div>
      <div className="flex-1 space-y-2 overflow-y-auto p-4">
        {loaded && list.length === 0 && (
          <div className="mt-24 text-center text-sm text-neutral-500">
            No downloads{filter !== "all" ? " in this view" : ""}. Add a URL or send one from the browser extension.
          </div>
        )}
        {list.map((p) => (
          <DownloadRow key={p.id} p={p} />
        ))}
      </div>
    </div>
  );
}
