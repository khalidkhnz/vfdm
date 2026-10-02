export type OfflinePolicy = "browser" | "block";

export interface ExtSettings {
  enabled: boolean;
  token: string;
  /** 0 = auto-scan the port range */
  port: number;
  /** Lower-cased extensions without dot. Empty = capture everything. */
  extensions: string[];
  minSizeMb: number;
  skipHosts: string[];
  offlinePolicy: OfflinePolicy;
  notify: boolean;
}

export const DEFAULT_EXTENSIONS = [
  "zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso", "img", "dmg", "pkg",
  "exe", "msi", "apk", "deb", "rpm", "appimage",
  "mp4", "mkv", "avi", "mov", "webm", "mp3", "flac", "wav", "m4a",
  "pdf", "epub", "bin", "safetensors", "gguf", "ckpt", "pt", "pth",
];

export const DEFAULTS: ExtSettings = {
  enabled: true,
  token: "",
  port: 0,
  extensions: DEFAULT_EXTENSIONS,
  minSizeMb: 0,
  skipHosts: [],
  offlinePolicy: "browser",
  notify: true,
};

const KEY = "settings";

export async function loadSettings(): Promise<ExtSettings> {
  const r = await chrome.storage.local.get(KEY);
  return { ...DEFAULTS, ...(r[KEY] as Partial<ExtSettings> | undefined) };
}

export async function saveSettings(s: ExtSettings): Promise<void> {
  await chrome.storage.local.set({ [KEY]: s });
}

export function onSettingsChanged(cb: (s: ExtSettings) => void): void {
  chrome.storage.onChanged.addListener((changes, area) => {
    if (area === "local" && changes[KEY]) {
      cb({ ...DEFAULTS, ...(changes[KEY].newValue as Partial<ExtSettings>) });
    }
  });
}
