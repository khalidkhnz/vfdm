/**
 * Wire format between the browser extension and the desktop app's loopback bridge.
 * Field names are snake_case to match the Rust `DownloadRequest` (serde defaults)
 * in crates/vfdm-engine/src/types.rs. Keep both in sync by hand.
 */

export const PROTOCOL_VERSION = 1;

export const PORT_RANGE = { start: 7800, end: 7810 } as const;

export interface DownloadRequest {
  url: string;
  original_url?: string;
  filename?: string;
  referrer?: string;
  user_agent?: string;
  cookies?: string;
  headers?: [string, string][];
  dest_dir?: string;
  max_connections?: number;
}

export interface PingResponse {
  app: "vfdm";
  version: string;
  protocol: number;
}

export interface DownloadResponse {
  id: number;
}

export const ENDPOINTS = {
  ping: "/ping",
  download: "/download",
} as const;

export function bridgeBase(port: number): string {
  return `http://127.0.0.1:${port}`;
}
