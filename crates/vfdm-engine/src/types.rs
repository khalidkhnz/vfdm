use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type DownloadId = u64;

pub const META_VERSION: u32 = 2;

/// How a download is fetched. `Auto` resolves to one of the others once the
/// URL or the probed Content-Type is known.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Auto,
    File,
    Hls,
    Dash,
    Ytdlp,
}

impl Kind {
    pub fn is_stream(self) -> bool {
        matches!(self, Self::Hls | Self::Dash)
    }
}

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
    #[serde(default)]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
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
    /// Post-download work: ffmpeg remux/mux, yt-dlp post-processing.
    Processing,
    Paused,
    Completed,
    Failed(String),
    Cancelled,
}

impl DownloadStatus {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Probing | Self::Downloading | Self::Processing)
    }
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackRole {
    Muxed,
    Video,
    Audio,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantInfo {
    pub uri: String,
    pub bandwidth: u64,
    pub resolution: Option<(u32, u32)>,
    pub codecs: Option<String>,
    pub audio_group: Option<String>,
}

/// One output file of a stream download. `segments_done` is the in-order
/// write cursor and the only resume state; `bytes_done` is advanced only
/// after the bytes hit the file, so `file_len >= bytes_done` always holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackState {
    pub role: TrackRole,
    pub part_path: PathBuf,
    pub segment_count: u32,
    pub segments_done: u32,
    pub bytes_done: u64,
    pub last_init: Option<u32>,
    pub fingerprint: String,
    /// Fetched but not yet written (UI only).
    #[serde(skip)]
    pub ready: Vec<u32>,
}

impl TrackState {
    pub fn is_done(&self) -> bool {
        self.segments_done >= self.segment_count
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamState {
    pub manifest_url: String,
    #[serde(default)]
    pub variants: Vec<VariantInfo>,
    pub chosen: Option<u32>,
    pub tracks: Vec<TrackState>,
    pub container_ext: String,
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
    #[serde(default)]
    pub kind: Kind,
    #[serde(default)]
    pub total_is_estimate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<StreamState>,
}

impl DownloadMeta {
    pub fn downloaded(&self) -> u64 {
        match &self.stream {
            Some(s) if self.kind.is_stream() => s.tracks.iter().map(|t| t.bytes_done).sum(),
            _ => self.segments.iter().map(|s| s.downloaded).sum(),
        }
    }

    pub fn all_done(&self) -> bool {
        !self.segments.is_empty() && self.segments.iter().all(|s| s.is_done())
    }

    /// (segment_count, segments_done) across tracks, for stream kinds.
    pub fn segment_counts(&self) -> Option<(u32, u32)> {
        let s = self.stream.as_ref()?;
        if !self.kind.is_stream() {
            return None;
        }
        Some((
            s.tracks.iter().map(|t| t.segment_count).sum(),
            s.tracks.iter().map(|t| t.segments_done).sum(),
        ))
    }

    /// Byte total for progress/ETA. Streams don't know their size up front, so
    /// after a few segments the average segment size is extrapolated.
    pub fn effective_total(&self) -> (Option<u64>, bool) {
        if let Some(t) = self.total {
            return (Some(t), self.total_is_estimate);
        }
        if let Some((count, done)) = self.segment_counts() {
            if done >= 3 && count > 0 {
                let bytes = self.downloaded();
                return (Some(bytes / done as u64 * count as u64), true);
            }
        }
        (None, false)
    }

    /// Every file or directory that holds in-progress data.
    pub fn partial_paths(&self) -> Vec<PathBuf> {
        let mut v = Vec::new();
        if !self.part_path.as_os_str().is_empty() {
            v.push(self.part_path.clone());
        }
        if let Some(s) = &self.stream {
            for t in &s.tracks {
                if !v.contains(&t.part_path) {
                    v.push(t.part_path.clone());
                }
            }
        }
        v
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentProgress {
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
}

/// Snapshot sent to UIs. Cheap to clone, serializable.
/// For stream kinds `segments` are in segment units over `segment_count`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub id: DownloadId,
    pub status: DownloadStatus,
    pub kind: Kind,
    pub url: String,
    pub filename: String,
    pub final_path: PathBuf,
    pub total: Option<u64>,
    pub total_is_estimate: bool,
    pub downloaded: u64,
    pub speed_bps: f64,
    pub eta_secs: Option<u64>,
    pub resumable: bool,
    pub segments: Vec<SegmentProgress>,
    pub segment_count: Option<u32>,
    pub segments_done: Option<u32>,
    pub note: Option<String>,
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
    /// Resolved by the host app; the engine never searches for binaries.
    #[serde(default)]
    pub ffmpeg_path: Option<PathBuf>,
    #[serde(default)]
    pub ytdlp_path: Option<PathBuf>,
    #[serde(default)]
    pub js_runtime: Option<PathBuf>,
}

impl Settings {
    pub fn new(download_dir: PathBuf) -> Self {
        Self {
            download_dir,
            max_connections: 8,
            max_concurrent: 3,
            ffmpeg_path: None,
            ytdlp_path: None,
            js_runtime: None,
        }
    }
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
