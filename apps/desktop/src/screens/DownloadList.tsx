import { useMemo } from "react";
import { Link2 } from "lucide-react";
import { matches, sortedDownloads, useDownloads } from "../store/downloads";
import DownloadRow from "../components/DownloadRow";
import { Kbd } from "../components/ui";

const EMPTY: Record<string, { title: string; body: string }> = {
  all: { title: "Nothing here yet", body: "Add a link, drop one onto this window, or capture downloads from the browser extension." },
  active: { title: "Nothing in progress", body: "New downloads start here." },
  done: { title: "No finished downloads", body: "Completed files show up here with Open and Show in folder." },
  failed: { title: "No failures", body: "Downloads that stop with an error land here so you can retry them." },
};

export default function DownloadList({ onAdd }: { onAdd: () => void }) {
  const items = useDownloads((s) => s.items);
  const loaded = useDownloads((s) => s.loaded);
  const filter = useDownloads((s) => s.filter);
  const list = useMemo(() => sortedDownloads(items).filter((p) => matches(p, filter)), [items, filter]);

  if (loaded && list.length === 0) {
    const e = EMPTY[filter];
    return (
      <div className="flex flex-1 flex-col items-center justify-center px-8 text-center">
        <Link2 size={22} className="text-dim" />
        <h3 className="mt-3 text-[15px] font-medium">{e.title}</h3>
        <p className="mt-1 max-w-sm text-[13px] text-mute">{e.body}</p>
        {filter === "all" && (
          <button onClick={onAdd} className="mt-4 flex items-center gap-2 text-[13px] text-accent hover:underline">
            Add a download <Kbd>⌘N</Kbd>
          </button>
        )}
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto">
      {list.map((p) => (
        <DownloadRow key={p.id} p={p} />
      ))}
    </div>
  );
}
