import { useState } from "react";
import { api, type Progress } from "../lib/ipc";
import { bytes, eta, percent, speed } from "../lib/format";
import SegmentBar from "./SegmentBar";

const STATUS_STYLE: Record<string, string> = {
  queued: "text-neutral-400 border-neutral-700",
  probing: "text-sky-300 border-sky-800",
  downloading: "text-sky-300 border-sky-800",
  processing: "text-violet-300 border-violet-800",
  paused: "text-amber-300 border-amber-800",
  completed: "text-emerald-300 border-emerald-800",
  failed: "text-rose-300 border-rose-800",
  cancelled: "text-neutral-500 border-neutral-700",
};

function Btn({ onClick, children, danger }: { onClick: () => void; children: React.ReactNode; danger?: boolean }) {
  return (
    <button
      onClick={onClick}
      className={`rounded px-2 py-1 text-xs transition ${
        danger ? "text-rose-300 hover:bg-rose-950" : "text-neutral-300 hover:bg-neutral-800"
      }`}
    >
      {children}
    </button>
  );
}

export default function DownloadRow({ p }: { p: Progress }) {
  const [confirmRemove, setConfirmRemove] = useState(false);
  const st = p.status.state;
  const active = st === "downloading" || st === "probing" || st === "processing";
  const isStream = p.kind === "hls" || p.kind === "dash";
  const pct = isStream && p.segment_count
    ? percent(p.segments_done ?? 0, p.segment_count)
    : percent(p.downloaded, p.total);

  return (
    <div className="group rounded-lg border border-neutral-800 bg-neutral-900/60 px-4 py-3 hover:border-neutral-700">
      <div className="flex items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-medium" title={p.final_path || p.url}>
              {p.filename || p.url}
            </span>
            <span className={`shrink-0 rounded border px-1.5 py-px text-[10px] uppercase tracking-wide ${STATUS_STYLE[st]}`}>
              {st}
            </span>
            {p.kind !== "file" && p.kind !== "auto" && (
              <span className="shrink-0 rounded border border-neutral-700 px-1.5 py-px text-[10px] uppercase tracking-wide text-neutral-400">
                {p.kind}
              </span>
            )}
            {!p.resumable && (st === "downloading" || st === "paused") && (
              <span className="shrink-0 text-[10px] text-neutral-500">not resumable</span>
            )}
          </div>
          <div className="mt-0.5 truncate text-xs text-neutral-500" title={p.url}>
            {p.url}
          </div>
        </div>
        <div className="flex shrink-0 gap-1 opacity-0 transition group-hover:opacity-100">
          {active && <Btn onClick={() => api.pause(p.id)}>Pause</Btn>}
          {st === "paused" && <Btn onClick={() => api.resume(p.id)}>Resume</Btn>}
          {st === "queued" && <Btn onClick={() => api.pause(p.id)}>Hold</Btn>}
          {(st === "failed" || st === "cancelled") && <Btn onClick={() => api.retry(p.id)}>Retry</Btn>}
          {(active || st === "paused" || st === "queued") && (
            <Btn danger onClick={() => api.cancel(p.id, true)}>Cancel</Btn>
          )}
          {st === "completed" && (
            <>
              <Btn onClick={() => api.openFile(p.id)}>Open</Btn>
              <Btn onClick={() => api.revealFile(p.id)}>Reveal</Btn>
            </>
          )}
          {!active && !confirmRemove && <Btn onClick={() => setConfirmRemove(true)}>Remove</Btn>}
          {confirmRemove && (
            <>
              <Btn onClick={() => api.remove(p.id, false)}>Keep file</Btn>
              <Btn danger onClick={() => api.remove(p.id, true)}>Delete file</Btn>
              <Btn onClick={() => setConfirmRemove(false)}>×</Btn>
            </>
          )}
        </div>
      </div>

      <div className="mt-2">
        <SegmentBar segments={p.segments} total={isStream ? p.segment_count : p.total} done={st === "completed"} />
      </div>

      <div className="mt-1.5 flex items-center gap-4 text-xs text-neutral-400 tabular-nums">
        <span>
          {bytes(p.downloaded)}
          {p.total ? ` / ${p.total_is_estimate ? "~" : ""}${bytes(p.total)}` : ""}
          {p.total || (isStream && p.segment_count) ? ` · ${pct.toFixed(1)}%` : ""}
        </span>
        {active && st !== "processing" && <span>{speed(p.speed_bps)}</span>}
        {active && p.eta_secs != null && <span>ETA {eta(p.eta_secs)}</span>}
        {isStream && p.segment_count != null && (
          <span>
            {p.segments_done ?? 0}/{p.segment_count} segments
          </span>
        )}
        {!isStream && active && p.segments.length > 1 && <span>{p.segments.length} connections</span>}
        {st === "failed" && <span className="text-rose-300">{p.status.state === "failed" ? p.status.message : ""}</span>}
        {p.note && st === "completed" && <span className="text-amber-300/80">{p.note}</span>}
      </div>
    </div>
  );
}
