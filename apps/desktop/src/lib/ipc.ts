import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { MockEngine, MOCK_BRIDGE, MOCK_TOOLS } from "./mock";

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

export interface Api {
  list(): Promise<Progress[]>;
  add(req: DownloadRequest): Promise<number>;
  pause(id: number): Promise<void>;
  resume(id: number): Promise<void>;
  retry(id: number): Promise<void>;
  cancel(id: number, deleteFiles: boolean): Promise<void>;
  remove(id: number, deleteFiles: boolean): Promise<void>;
  openFile(id: number): Promise<void>;
  revealFile(id: number): Promise<void>;
  getSettings(): Promise<AppSettings>;
  setSettings(settings: AppSettings): Promise<AppSettings>;
  getBridgeInfo(): Promise<BridgeInfo>;
  regenerateToken(): Promise<BridgeInfo>;
  pickDownloadDir(): Promise<string | null>;
  getTools(): Promise<ToolStatus[]>;
  refreshTools(): Promise<ToolStatus[]>;
  installTool(name: ToolName): Promise<ToolStatus>;
  pickToolPath(): Promise<string | null>;
}

export interface Events {
  onProgress(cb: (batch: Progress[]) => void): Promise<UnlistenFn>;
  onAdded(cb: (p: Progress) => void): Promise<UnlistenFn>;
  onRemoved(cb: (id: number) => void): Promise<UnlistenFn>;
  onStatus(cb: (id: number, status: DownloadStatus) => void): Promise<UnlistenFn>;
  onBridgeRequest(cb: (p: Progress) => void): Promise<UnlistenFn>;
  onToolsChanged(cb: (t: ToolStatus[]) => void): Promise<UnlistenFn>;
}

const tauriApi: Api = {
  list: () => invoke<Progress[]>("list_downloads"),
  add: (req) => invoke<number>("add_download", { req }),
  pause: (id) => invoke<void>("pause_download", { id }),
  resume: (id) => invoke<void>("resume_download", { id }),
  retry: (id) => invoke<void>("retry_download", { id }),
  cancel: (id, deleteFiles) => invoke<void>("cancel_download", { id, deleteFiles }),
  remove: (id, deleteFiles) => invoke<void>("remove_download", { id, deleteFiles }),
  openFile: (id) => invoke<void>("open_file", { id }),
  revealFile: (id) => invoke<void>("reveal_file", { id }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  setSettings: (settings) => invoke<AppSettings>("set_settings", { settings }),
  getBridgeInfo: () => invoke<BridgeInfo>("get_bridge_info"),
  regenerateToken: () => invoke<BridgeInfo>("regenerate_token"),
  pickDownloadDir: () => invoke<string | null>("pick_download_dir"),
  getTools: () => invoke<ToolStatus[]>("get_tools"),
  refreshTools: () => invoke<ToolStatus[]>("refresh_tools"),
  installTool: (name) => invoke<ToolStatus>("install_tool", { name }),
  pickToolPath: () => invoke<string | null>("pick_tool_path"),
};

const tauriEvents: Events = {
  onProgress: (cb) => listen<Progress[]>("download://progress", (e) => cb(e.payload)),
  onAdded: (cb) => listen<Progress>("download://added", (e) => cb(e.payload)),
  onRemoved: (cb) => listen<number>("download://removed", (e) => cb(e.payload)),
  onStatus: (cb) =>
    listen<{ id: number; status: DownloadStatus }>("download://status", (e) => cb(e.payload.id, e.payload.status)),
  onBridgeRequest: (cb) => listen<Progress>("bridge://request", (e) => cb(e.payload)),
  onToolsChanged: (cb) => listen<ToolStatus[]>("tools://changed", (e) => cb(e.payload)),
};

export const IN_TAURI = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Browser-only simulator (see mock.ts) so the UI runs without the Tauri shell. */
function mockBindings(): { api: Api; events: Events } {
  const eng = new MockEngine();
  const ok = <T,>(v: T) => Promise.resolve(v);
  const api: Api = {
    list: () => ok(eng.list()),
    add: (req) => ok(eng.add(req)),
    pause: (id) => ok(eng.pause(id)),
    resume: (id) => ok(eng.resume(id)),
    retry: (id) => ok(eng.resume(id)),
    cancel: (id) => ok(eng.cancel(id)),
    remove: (id) => ok(eng.remove(id)),
    openFile: () => ok(undefined),
    revealFile: () => ok(undefined),
    getSettings: () => ok(eng.settings),
    setSettings: (s) => ok((eng.settings = s)),
    getBridgeInfo: () => ok(MOCK_BRIDGE),
    regenerateToken: () => ok(MOCK_BRIDGE),
    pickDownloadDir: () => ok(null),
    getTools: () => ok(MOCK_TOOLS),
    refreshTools: () => ok(MOCK_TOOLS),
    installTool: (name) =>
      new Promise((r) =>
        setTimeout(
          () => r({ ...MOCK_TOOLS.find((t) => t.name === name)!, path: "/app/tools/" + name, source: "bundled" }),
          1200,
        ),
      ),
    pickToolPath: () => ok(null),
  };
  const events: Events = {
    onProgress: (cb) => ok(eng.progress.on(cb)),
    onAdded: (cb) => ok(eng.added.on(cb)),
    onRemoved: (cb) => ok(eng.removed.on(cb)),
    onStatus: (cb) => ok(eng.status.on((e) => cb(e.id, e.status))),
    onBridgeRequest: (cb) => ok(eng.bridge.on(cb)),
    onToolsChanged: () => ok(() => {}),
  };
  return { api, events };
}

const bindings = IN_TAURI ? { api: tauriApi, events: tauriEvents } : mockBindings();
export const api: Api = bindings.api;
export const events: Events = bindings.events;
