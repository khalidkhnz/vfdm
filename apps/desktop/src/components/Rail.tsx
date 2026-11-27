import { useEffect, useMemo, useState } from "react";
import { Plus, Settings2 } from "lucide-react";
import { api, events, type BridgeInfo, type ToolStatus } from "../lib/ipc";
import { countsOf, useDownloads, type Filter } from "../store/downloads";
import { Kbd } from "./ui";

const FILTERS: { key: Filter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "active", label: "Active" },
  { key: "done", label: "Done" },
  { key: "failed", label: "Failed" },
];

function Dot({ ok, pending }: { ok: boolean; pending?: boolean }) {
  return (
    <span
      className={`inline-block h-1.5 w-1.5 rounded-full ${pending ? "bg-dim" : ok ? "bg-ok" : "bg-error"}`}
      aria-hidden
    />
  );
}

export default function Rail({
  onAdd,
  onSettings,
  compact,
}: {
  onAdd: () => void;
  onSettings: () => void;
  compact: boolean;
}) {
  const filter = useDownloads((s) => s.filter);
  const setFilter = useDownloads((s) => s.setFilter);
  const items = useDownloads((s) => s.items);
  const counts = useMemo(() => countsOf(items), [items]);
  const [tools, setTools] = useState<ToolStatus[] | null>(null);
  const [bridge, setBridge] = useState<BridgeInfo | null>(null);

  useEffect(() => {
    api.getTools().then(setTools).catch(() => setTools([]));
    const un = events.onToolsChanged(setTools);
    let alive = true;
    const poll = () => api.getBridgeInfo().then((b) => alive && setBridge(b)).catch(() => {});
    poll();
    const t = setInterval(poll, 5000);
    return () => {
      alive = false;
      clearInterval(t);
      void un.then((f) => f());
    };
  }, []);

  const tool = (n: ToolStatus["name"]) => tools?.find((t) => t.name === n);
  const status = [
    { label: "ffmpeg", ok: !!tool("ffmpeg")?.path, pending: tools === null },
    { label: "yt-dlp", ok: !!tool("ytdlp")?.path, pending: tools === null },
    { label: "Browser bridge", ok: !!bridge && bridge.port > 0, pending: bridge === null },
  ];

  return (
    <nav className={`flex h-full flex-col border-r border-line bg-surface ${compact ? "w-14" : "w-44"}`}>
      <div className="px-2 pt-3">
        <button
          onClick={onAdd}
          className="flex h-9 w-full items-center gap-2 rounded-md bg-accent px-2.5 text-[13px] font-medium text-bg transition-colors hover:bg-accent-hi focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
          title="Add download (⌘N)"
        >
          <Plus size={16} strokeWidth={2.5} />
          {!compact && (
            <>
              <span>Add</span>
              <span className="ml-auto font-mono text-[11px] text-bg/70">⌘N</span>
            </>
          )}
        </button>
      </div>

      <ul className="mt-3 flex flex-col gap-px px-2">
        {FILTERS.map((f) => {
          const on = filter === f.key;
          return (
            <li key={f.key}>
              <button
                onClick={() => setFilter(f.key)}
                aria-current={on ? "page" : undefined}
                className={`flex h-8 w-full items-center rounded-md px-2.5 text-[13px] transition-colors focus-visible:outline-2 focus-visible:outline-accent ${
                  on ? "bg-raised text-ink" : "text-mute hover:text-ink"
                }`}
                title={f.label}
              >
                {compact ? (
                  <span className="font-mono text-[12px] tabular-nums">{counts[f.key]}</span>
                ) : (
                  <>
                    <span>{f.label}</span>
                    <span className={`ml-auto font-mono text-[12px] tabular-nums ${on ? "text-mute" : "text-dim"}`}>{counts[f.key]}</span>
                  </>
                )}
              </button>
            </li>
          );
        })}
      </ul>

      <div className="mt-auto px-2 pb-3">
        {!compact && (
          <ul className="mb-2 flex flex-col gap-1 px-2.5 text-[12px] text-mute">
            {status.map((s) => (
              <li key={s.label} className="flex items-center gap-2">
                <Dot ok={s.ok} pending={s.pending} />
                <span>{s.label}</span>
              </li>
            ))}
          </ul>
        )}
        <button
          onClick={onSettings}
          className="flex h-8 w-full items-center gap-2 rounded-md px-2.5 text-[13px] text-mute transition-colors hover:bg-raised hover:text-ink focus-visible:outline-2 focus-visible:outline-accent"
          title="Settings (⌘,)"
        >
          <Settings2 size={15} />
          {!compact && (
            <>
              <span>Settings</span>
              <Kbd>⌘,</Kbd>
            </>
          )}
        </button>
      </div>
    </nav>
  );
}
