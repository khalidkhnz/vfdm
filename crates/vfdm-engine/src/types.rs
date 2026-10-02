use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type DownloadId = u64;

pub const META_VERSION: u32 = 1;

/// What a caller (UI, extension bridge) asks the engine to fetch.
/// Field names are the wire format shared with `@vfdm/protocol`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referrer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cookies: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dest_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_connections: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub id: u32,
    /// Inclusive, fixed for the segment's lifetime.
    pub start: u64,
    /// Inclusive. Shrinks when another worker steals the tail.
    pub end: u64,
    /// Bytes written from `start`; next byte to fetch is `start + downloaded`.
    pub downloaded: u64,
}

impl Segment {
    pub fn next(&self) -> u64 {
        self.start + self.downloaded
    }
    pub fn len(&self) -> u64 {
        self.end - self.start + 1
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn remaining(&self) -> u64 {
        (self.end + 1).saturating_sub(self.next())
    }
    pub fn is_done(&self) -> bool {
        self.remaining() == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "message", rename_all = "snake_case")]
pub enum DownloadStatus {
    Queued,
    Probing,
    Downloading,
    Paused,
    Completed,
    Failed(String),
    Cancelled,
}

impl DownloadStatus {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Probing | Self::Downloading)
    }
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

/// Everything persisted per download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadMeta {
    pub version: u32,
    pub id: DownloadId,
    pub request: DownloadRequest,
    pub final_url: String,
    pub filename: String,
    pub part_path: PathBuf,
    pub final_path: PathBuf,
    pub total: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub resumable: bool,
    pub segments: Vec<Segment>,
    pub status: DownloadStatus,
    pub created_at: u64,
    pub completed_at: Option<u64>,
}

impl DownloadMeta {
    pub fn downloaded(&self) -> u64 {
        self.segments.iter().map(|s| s.downloaded).sum()
    }
    pub fn all_done(&self) -> bool {
        !self.segments.is_empty() && self.segments.iter().all(|s| s.is_done())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentProgress {
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
}

/// Snapshot sent to UIs. Cheap to clone, serializable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub id: DownloadId,
    pub status: DownloadStatus,
    pub url: String,
    pub filename: String,
    pub final_path: PathBuf,
    pub total: Option<u64>,
    pub downloaded: u64,
    pub speed_bps: f64,
    pub eta_secs: Option<u64>,
    pub resumable: bool,
    pub segments: Vec<SegmentProgress>,
    pub created_at: u64,
    pub completed_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DownloadEvent {
    Added(Progress),
    Progress(Progress),
    Status {
        id: DownloadId,
        status: DownloadStatus,
    },
    Removed {
        id: DownloadId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub download_dir: PathBuf,
    pub max_connections: u8,
    pub max_concurrent: u8,
}

impl Settings {
    pub fn new(download_dir: PathBuf) -> Self {
        Self {
            download_dir,
            max_connections: 8,
            max_concurrent: 3,
        }
    }
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
