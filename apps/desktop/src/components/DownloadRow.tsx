import { useState } from "react";
import {
  Check,
  Clapperboard,
  File,
  FolderOpen,
  Pause,
  Play,
  RotateCcw,
  Trash2,
  X,
  MonitorPlay,
} from "lucide-react";
import { api, type Kind, type Progress } from "../lib/ipc";
import { bytes, eta, percent, speed } from "../lib/format";
import BlockMap from "./BlockMap";
import { IconButton } from "./ui";

const DOT: Record<string, string> = {
  queued: "bg-dim",
  probing: "bg-accent",
  downloading: "bg-accent",
  processing: "bg-accent",
  paused: "bg-warn",
  completed: "bg-ok",
  failed: "bg-error",
  cancelled: "bg-dim",
};

const WORD: Record<string, string> = {
  queued: "Waiting",
  probing: "Connecting",
  downloading: "Downloading",
  processing: "Finishing",
  paused: "Paused",
  completed: "Done",
  failed: "Failed",
  cancelled: "Cancelled",
};

function KindIcon({ kind }: { kind: Kind }) {
  const cls = "text-dim";
  if (kind === "hls" || kind === "dash") return <Clapperboard size={16} className={cls} />;
  if (kind === "ytdlp") return <MonitorPlay size={16} className={cls} />;
  return <File size={16} className={cls} />;
}

function kindWord(kind: Kind): string | null {
  return kind === "hls" ? "HLS" : kind === "dash" ? "DASH" : kind === "ytdlp" ? "yt-dlp" : null;
}

function ago(ms: number): string {
  const s = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}

export default function DownloadRow({ p }: { p: Progress }) {
  const [confirm, setConfirm] = useState(false);
  const st = p.status.state;
  const active = st === "downloading" || st === "probing" || st === "processing";
  const isStream = p.kind === "hls" || p.kind === "dash";
  const showMap = active || st === "paused" || st === "queued";
  const pct = isStream && p.segment_count ? percent(p.segments_done ?? 0, p.segment_count) : percent(p.downloaded, p.total);

  const actions = (
    <div className="flex items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
      {active && st !== "processing" && (
        <IconButton label="Pause" onClick={() => api.pause(p.id)}>
          <Pause size={15} />
        </IconButton>
      )}
      {(st === "paused" || st === "queued") && (
        <IconButton label={st === "queued" ? "Hold" : "Resume"} onClick={() => (st === "queued" ? api.pause(p.id) : api.resume(p.id))}>
          {st === "queued" ? <Pause size={15} /> : <Play size={15} />}
        </IconButton>
      )}
      {(st === "failed" || st === "cancelled") && (
        <IconButton label="Retry" onClick={() => api.retry(p.id)}>
          <RotateCcw size={15} />
        </IconButton>
      )}
      {st === "completed" && (
        <IconButton label="Show in folder" onClick={() => api.revealFile(p.id)}>
          <FolderOpen size={15} />
        </IconButton>
      )}
      {(active || st === "paused" || st === "queued") && (
        <IconButton label="Cancel and delete partial file" danger onClick={() => api.cancel(p.id, true)}>
          <X size={15} />
        </IconButton>
      )}
      {!active && !confirm && (
        <IconButton label="Remove from list" danger onClick={() => setConfirm(true)}>
          <Trash2 size={15} />
        </IconButton>
      )}
      {confirm && (
        <div className="flex items-center gap-1 pl-1 text-[12px]">
          <button onClick={() => api.remove(p.id, false)} className="rounded px-2 py-1 text-mute hover:bg-raised hover:text-ink">
            Keep file
          </button>
          <button onClick={() => api.remove(p.id, true)} className="rounded px-2 py-1 text-error hover:bg-error/10">
            Delete file
          </button>
          <IconButton label="Never mind" onClick={() => setConfirm(false)}>
            <X size={13} />
          </IconButton>
        </div>
      )}
    </div>
  );

  if (st === "completed") {
    return (
      <article className="group flex items-center gap-3 border-b border-line px-5 py-2.5 hover:bg-surface/70">
        <Check size={16} className="text-ok" />
        <button
          onClick={() => api.openFile(p.id)}
          className="min-w-0 flex-1 truncate text-left text-[13.5px] hover:underline"
          title={p.final_path}
        >
          {p.filename}
        </button>
        {p.note && <span className="hidden truncate text-[12px] text-warn/90 md:block">{p.note}</span>}
        <span className="font-mono text-[12px] tabular-nums text-dim">
          {bytes(p.total ?? p.downloaded)}
          {"   "}
          {ago(p.completed_at ?? p.created_at)}
        </span>
        {actions}
      </article>
    );
  }

  return (
    <article className="group border-b border-line px-5 py-3 hover:bg-surface/70">
      <div className="flex items-center gap-3">
        <KindIcon kind={p.kind} />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            <span className="truncate text-[14px] font-medium" title={p.final_path || p.url}>
              {p.filename || p.url}
            </span>
            {kindWord(p.kind) && <span className="shrink-0 font-mono text-[11px] text-dim">{kindWord(p.kind)}</span>}
          </div>
          <div className="truncate text-[12px] text-dim" title={p.url}>
            {p.url}
          </div>
        </div>
        <div className="shrink-0 text-right font-mono text-[12.5px] tabular-nums text-mute">
          <div>
            <span className="text-ink">{bytes(p.downloaded)}</span>
            {p.total ? ` / ${p.total_is_estimate ? "~" : ""}${bytes(p.total)}` : ""}
          </div>
          {(p.total || (isStream && p.segment_count)) && <div className="text-dim">{pct.toFixed(1)}%</div>}
        </div>
        {actions}
      </div>

      {showMap && (
        <div className="mt-2.5 pl-7">
          <BlockMap p={p} />
        </div>
      )}

      <div className="mt-2 flex items-center gap-4 pl-7 font-mono text-[12px] tabular-nums text-mute">
        <span className="flex items-center gap-1.5">
          <span className={`h-1.5 w-1.5 rounded-full ${DOT[st]}`} />
          {WORD[st]}
        </span>
        {st === "downloading" && <span>{speed(p.speed_bps)}</span>}
        {st === "downloading" && p.eta_secs != null && <span>{eta(p.eta_secs)} left</span>}
        {isStream && p.segment_count != null && (
          <span>
            {p.segments_done ?? 0}/{p.segment_count} segments
          </span>
        )}
        {!isStream && active && p.segments.length > 1 && <span>{p.segments.length} connections</span>}
        {!p.resumable && (st === "downloading" || st === "paused") && <span className="text-dim">not resumable</span>}
        {st === "failed" && <span className="truncate text-error">{p.status.state === "failed" ? p.status.message : ""}</span>}
      </div>
    </article>
  );
}
