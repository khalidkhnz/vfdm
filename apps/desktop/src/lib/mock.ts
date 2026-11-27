/**
 * Browser-only simulator so the UI can be developed and screenshotted
 * outside Tauri. Mirrors the engine's behaviour closely enough to render:
 * segmented files with work stealing, HLS/DASH segment counts, yt-dlp,
 * and the terminal states. Selected in ipc.ts when __TAURI_INTERNALS__ is absent.
 */
import type {
  AppSettings,
  BridgeInfo,
  DownloadRequest,
  DownloadStatus,
  Progress,
  SegmentProgress,
  ToolStatus,
} from "./ipc";

type Listener<T> = (payload: T) => void;

class Emitter<T> {
  private ls = new Set<Listener<T>>();
  on(l: Listener<T>): () => void {
    this.ls.add(l);
    return () => this.ls.delete(l);
  }
  emit(p: T) {
    for (const l of this.ls) l(p);
  }
}

interface Sim extends Progress {
  rate: number;
  tickBias: number;
}

const MB = 1024 * 1024;
let nextId = 1;
const now = () => Date.now();

function file(name: string, total: number, status: DownloadStatus["state"], opts: Partial<Sim> = {}): Sim {
  const n = 8;
  const size = Math.floor(total / n);
  const segments: SegmentProgress[] = Array.from({ length: n }, (_, i) => ({
    start: i * size,
    end: i === n - 1 ? total - 1 : (i + 1) * size - 1,
    downloaded: 0,
  }));
  return {
    id: nextId++,
    status: { state: status } as DownloadStatus,
    kind: "file",
    url: `https://releases.example.com/${name}`,
    filename: name,
    final_path: `/Users/you/Downloads/${name}`,
    total,
    total_is_estimate: false,
    downloaded: 0,
    speed_bps: 0,
    eta_secs: null,
    resumable: true,
    segments,
    segment_count: null,
    segments_done: null,
    note: null,
    created_at: now() - Math.floor(Math.random() * 3_600_000),
    completed_at: null,
    rate: 6 * MB,
    tickBias: 1,
    ...opts,
  };
}

function stream(name: string, kind: "hls" | "dash", count: number, status: DownloadStatus["state"]): Sim {
  return {
    ...file(name, 0, status),
    kind,
    url: kind === "hls" ? "https://cdn.example.com/v/ep12/index.m3u8" : "https://cdn.example.com/dash/manifest.mpd",
    total: null,
    total_is_estimate: true,
    segments: [],
    segment_count: count,
    segments_done: 0,
    rate: 3 * MB,
  };
}

function seed(): Sim[] {
  const a = file("ubuntu-24.04.2-desktop-amd64.iso", 5_800 * MB, "downloading");
  a.segments.forEach((s, i) => (s.downloaded = Math.floor((s.end - s.start) * [0.9, 0.6, 0.72, 0.4, 0.55, 0.3, 0.8, 0.1][i])));
  const b = stream("ep12.mp4", "hls", 181, "downloading");
  b.segments_done = 64;
  const c = stream("Angel One.mp4", "dash", 196, "queued");
  const d: Sim = { ...file("Me at the zoo [jNQXAC9IVRw].webm", 48 * MB, "downloading"), kind: "ytdlp", url: "https://www.youtube.com/watch?v=jNQXAC9IVRw", total_is_estimate: true, segments: [{ start: 0, end: 48 * MB - 1, downloaded: 12 * MB }], rate: 1.2 * MB };
  const e = file("Blender_4.2.dmg", 310 * MB, "paused");
  e.segments.forEach((s, i) => (s.downloaded = i < 5 ? s.end - s.start + 1 : Math.floor((s.end - s.start) * 0.35)));
  const f = file("node-v22.12.0.pkg", 71 * MB, "completed");
  f.segments.forEach((s) => (s.downloaded = s.end - s.start + 1));
  f.completed_at = now() - 400_000;
  const g = file("dataset-shard-07.tar.zst", 2_300 * MB, "failed");
  g.status = { state: "failed", message: "server returned HTTP 403" };
  g.segments.forEach((s) => (s.downloaded = Math.floor((s.end - s.start) * 0.2)));
  const h = file("Fedora-Workstation-41.iso", 2_200 * MB, "queued");
  return [a, b, c, d, e, f, g, h];
}

export class MockEngine {
  items: Sim[];
  progress = new Emitter<Progress[]>();
  status = new Emitter<{ id: number; status: DownloadStatus }>();
  added = new Emitter<Progress>();
  removed = new Emitter<number>();
  bridge = new Emitter<Progress>();
  settings: AppSettings = {
    download_dir: "/Users/you/Downloads",
    max_connections: 8,
    max_concurrent: 3,
    notify_on_complete: true,
    focus_on_capture: true,
    ffmpeg_path: null,
    ytdlp_path: null,
    hls_variant: "best",
  };

  constructor() {
    const empty = new URLSearchParams(location.search).has("empty");
    this.items = empty ? [] : seed();
    for (const s of this.items) s.downloaded = sum(s);
    setInterval(() => this.tick(), 250);
    if (!empty) setTimeout(() => this.captured("firmware-update-3.1.bin", 92 * MB), 2500);
  }

  private captured(name: string, total: number) {
    const s = file(name, total, "queued");
    s.url = `https://vendor.example.com/dl/${name}`;
    s.created_at = now();
    this.items.unshift(s);
    this.added.emit(strip(s));
    this.bridge.emit(strip(s));
  }

  private setStatus(s: Sim, state: DownloadStatus) {
    s.status = state;
    this.status.emit({ id: s.id, status: state });
  }

  private tick() {
    const active = this.items.filter((s) => s.status.state === "downloading");
    const queued = this.items.filter((s) => s.status.state === "queued");
    if (active.length < this.settings.max_concurrent && queued.length) this.setStatus(queued[0], { state: "downloading" });
    const batch: Progress[] = [];
    for (const s of this.items) {
      if (s.status.state !== "downloading") continue;
      const bytes = (s.rate * 0.25) * (0.7 + Math.random() * 0.6);
      if (s.kind === "file" || s.kind === "ytdlp") {
        const live = s.segments.filter((g) => g.downloaded < g.end - g.start + 1);
        if (live.length === 0) {
          this.finish(s);
          continue;
        }
        const per = bytes / live.length;
        for (const g of live) {
          const cap = g.end - g.start + 1 - g.downloaded;
          g.downloaded += Math.min(cap, per * (0.5 + Math.random()));
        }
        // Work stealing: a finished worker takes the back half of the largest remainder.
        if (s.kind === "file" && live.length < 8) {
          const victim = live.reduce((m, g) => (g.end - g.start + 1 - g.downloaded > m.end - m.start + 1 - m.downloaded ? g : m), live[0]);
          const remaining = victim.end - victim.start + 1 - victim.downloaded;
          if (remaining > 64 * MB) {
            const mid = victim.start + victim.downloaded + Math.floor(remaining / 2);
            s.segments.push({ start: mid, end: victim.end, downloaded: 0 });
            victim.end = mid - 1;
          }
        }
        s.downloaded = sum(s);
        s.speed_bps = bytes * 4;
        s.eta_secs = s.total ? Math.ceil((s.total - s.downloaded) / s.speed_bps) : null;
      } else {
        const count = s.segment_count ?? 0;
        const step = Math.random() < 0.6 ? 1 : 2;
        s.segments_done = Math.min(count, (s.segments_done ?? 0) + step);
        s.downloaded += bytes;
        s.speed_bps = bytes * 4;
        const done = s.segments_done;
        s.total = done >= 3 ? Math.floor((s.downloaded / done) * count) : null;
        s.eta_secs = s.total ? Math.ceil((s.total - s.downloaded) / s.speed_bps) : null;
        s.segments = [
          { start: 0, end: count - 1, downloaded: done },
          ...[1, 2, 4].filter((k) => done + k < count).map((k) => ({ start: done + k, end: done + k, downloaded: 1 })),
        ];
        if (done >= count) {
          this.setStatus(s, { state: "processing" });
          setTimeout(() => this.finish(s), 1500);
          continue;
        }
      }
      batch.push(strip(s));
    }
    if (batch.length) this.progress.emit(batch);
  }

  private finish(s: Sim) {
    s.speed_bps = 0;
    s.eta_secs = null;
    s.completed_at = now();
    if (s.kind === "hls") {
      s.filename = s.filename.replace(/\.[^.]+$/, ".mp4");
      s.total = s.downloaded;
      s.total_is_estimate = false;
    }
    if (s.kind === "dash") s.note = "Saved video and audio separately — install ffmpeg (Settings → Tools) to mux them";
    this.setStatus(s, { state: "completed" });
    this.progress.emit([strip(s)]);
  }

  list(): Progress[] {
    return this.items.map(strip);
  }

  add(req: DownloadRequest): number {
    const name = req.filename || req.url.split("/").pop()?.split("?")[0] || "download";
    const s =
      req.kind === "hls" || req.kind === "dash"
        ? stream(name.replace(/\.(m3u8|mpd)$/, ".mp4"), req.kind, 120, "queued")
        : file(name, (200 + Math.random() * 2000) * MB, "queued");
    s.url = req.url;
    if (req.kind === "ytdlp") s.kind = "ytdlp";
    s.created_at = now();
    this.items.unshift(s);
    this.added.emit(strip(s));
    return s.id;
  }

  get(id: number) {
    return this.items.find((s) => s.id === id);
  }

  pause(id: number) {
    const s = this.get(id);
    if (s && (s.status.state === "downloading" || s.status.state === "queued")) {
      s.speed_bps = 0;
      s.eta_secs = null;
      this.setStatus(s, { state: "paused" });
    }
  }
  resume(id: number) {
    const s = this.get(id);
    if (s && ["paused", "failed", "cancelled"].includes(s.status.state)) this.setStatus(s, { state: "queued" });
  }
  cancel(id: number) {
    const s = this.get(id);
    if (s) {
      s.speed_bps = 0;
      this.setStatus(s, { state: "cancelled" });
    }
  }
  remove(id: number) {
    this.items = this.items.filter((s) => s.id !== id);
    this.removed.emit(id);
  }
}

function sum(s: Sim): number {
  return s.segments.reduce((a, g) => a + g.downloaded, 0);
}

function strip(s: Sim): Progress {
  const { rate: _r, tickBias: _b, ...p } = s;
  return { ...p, segments: p.segments.map((g) => ({ ...g })) };
}

export const MOCK_TOOLS: ToolStatus[] = [
  { name: "ffmpeg", path: "/opt/homebrew/bin/ffmpeg", version: "7.1", source: "path", installable: true, hint: null },
  { name: "ytdlp", path: null, version: null, source: "missing", installable: true, hint: null },
  { name: "node", path: "/opt/homebrew/bin/node", version: "22.12.0", source: "path", installable: false, hint: null },
];

export const MOCK_BRIDGE: BridgeInfo = { port: 7800, token: "3f9a1c0b7e2d4a6f8b1c3d5e7f9a0b2c4d6e8f0a1b3c5d7e9f1a3b5c7d9e1f2a" };
