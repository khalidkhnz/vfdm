import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type DownloadStatus =
  | { state: "queued" | "probing" | "downloading" | "processing" | "paused" | "completed" | "cancelled" }
  | { state: "failed"; message: string };

export type Kind = "auto" | "file" | "hls" | "dash" | "ytdlp";

export interface SegmentProgress {
  start: number;
  end: number;
  downloaded: number;
}

export interface Progress {
  id: number;
  status: DownloadStatus;
  kind: Kind;
  url: string;
  filename: string;
  final_path: string;
  total: number | null;
  total_is_estimate: boolean;
  downloaded: number;
  speed_bps: number;
  eta_secs: number | null;
  resumable: boolean;
  segments: SegmentProgress[];
  segment_count: number | null;
  segments_done: number | null;
  note: string | null;
  created_at: number;
  completed_at: number | null;
}

export interface DownloadRequest {
  url: string;
  filename?: string;
  dest_dir?: string;
  max_connections?: number;
  kind?: Kind;
}

export interface AppSettings {
  download_dir: string;
  max_connections: number;
  max_concurrent: number;
  notify_on_complete: boolean;
  focus_on_capture: boolean;
  ffmpeg_path: string | null;
  ytdlp_path: string | null;
  hls_variant: "best" | "ask";
}

export type ToolName = "ffmpeg" | "ytdlp" | "node";

export interface ToolStatus {
  name: ToolName;
  path: string | null;
  version: string | null;
  source: "override" | "bundled" | "path" | "missing";
  installable: boolean;
  hint: string | null;
}

export interface BridgeInfo {
  port: number;
  token: string;
}

export const api = {
  list: () => invoke<Progress[]>("list_downloads"),
  add: (req: DownloadRequest) => invoke<number>("add_download", { req }),
  pause: (id: number) => invoke<void>("pause_download", { id }),
  resume: (id: number) => invoke<void>("resume_download", { id }),
  retry: (id: number) => invoke<void>("retry_download", { id }),
  cancel: (id: number, deleteFiles: boolean) => invoke<void>("cancel_download", { id, deleteFiles }),
  remove: (id: number, deleteFiles: boolean) => invoke<void>("remove_download", { id, deleteFiles }),
  openFile: (id: number) => invoke<void>("open_file", { id }),
  revealFile: (id: number) => invoke<void>("reveal_file", { id }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  setSettings: (settings: AppSettings) => invoke<AppSettings>("set_settings", { settings }),
  getBridgeInfo: () => invoke<BridgeInfo>("get_bridge_info"),
  regenerateToken: () => invoke<BridgeInfo>("regenerate_token"),
  pickDownloadDir: () => invoke<string | null>("pick_download_dir"),
  getTools: () => invoke<ToolStatus[]>("get_tools"),
  refreshTools: () => invoke<ToolStatus[]>("refresh_tools"),
  installTool: (name: ToolName) => invoke<ToolStatus>("install_tool", { name }),
  pickToolPath: () => invoke<string | null>("pick_tool_path"),
};

export const events = {
  onProgress: (cb: (batch: Progress[]) => void): Promise<UnlistenFn> =>
    listen<Progress[]>("download://progress", (e) => cb(e.payload)),
  onAdded: (cb: (p: Progress) => void): Promise<UnlistenFn> =>
    listen<Progress>("download://added", (e) => cb(e.payload)),
  onRemoved: (cb: (id: number) => void): Promise<UnlistenFn> =>
    listen<number>("download://removed", (e) => cb(e.payload)),
  onStatus: (cb: (id: number, status: DownloadStatus) => void): Promise<UnlistenFn> =>
    listen<{ id: number; status: DownloadStatus }>("download://status", (e) =>
      cb(e.payload.id, e.payload.status),
    ),
  onBridgeRequest: (cb: (p: Progress) => void): Promise<UnlistenFn> =>
    listen<Progress>("bridge://request", (e) => cb(e.payload)),
  onToolsChanged: (cb: (t: ToolStatus[]) => void): Promise<UnlistenFn> =>
    listen<ToolStatus[]>("tools://changed", (e) => cb(e.payload)),
};
