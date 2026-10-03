use crate::tools::{ToolName, ToolStatus};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use vfdm_engine::Settings as EngineSettings;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HlsVariantPolicy {
    #[default]
    Best,
    Ask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub download_dir: PathBuf,
    pub max_connections: u8,
    pub max_concurrent: u8,
    pub notify_on_complete: bool,
    pub focus_on_capture: bool,
    /// Explicit binary paths; empty = auto-detect.
    #[serde(default)]
    pub ffmpeg_path: Option<PathBuf>,
    #[serde(default)]
    pub ytdlp_path: Option<PathBuf>,
    #[serde(default)]
    pub hls_variant: HlsVariantPolicy,
}

impl AppSettings {
    fn defaults(download_dir: PathBuf) -> Self {
        Self {
            download_dir,
            max_connections: 8,
            max_concurrent: 3,
            notify_on_complete: true,
            focus_on_capture: true,
            ffmpeg_path: None,
            ytdlp_path: None,
            hls_variant: HlsVariantPolicy::Best,
        }
    }

    pub fn load(data_dir: &Path, default_download_dir: PathBuf) -> Self {
        let path = data_dir.join("settings.json");
        std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| Self::defaults(default_download_dir))
            .clamped()
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let path = data_dir.join("settings.json");
        let tmp = data_dir.join("settings.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }

    pub fn clamped(mut self) -> Self {
        self.max_connections = self.max_connections.clamp(1, 16);
        self.max_concurrent = self.max_concurrent.clamp(1, 10);
        let empty = |p: &Option<PathBuf>| p.as_ref().is_some_and(|p| p.as_os_str().is_empty());
        if empty(&self.ffmpeg_path) {
            self.ffmpeg_path = None;
        }
        if empty(&self.ytdlp_path) {
            self.ytdlp_path = None;
        }
        self
    }

    pub fn to_engine(&self, tools: &[ToolStatus]) -> EngineSettings {
        let path_of = |n: ToolName| {
            tools
                .iter()
                .find(|t| t.name == n)
                .and_then(|t| t.path.clone())
        };
        EngineSettings {
            download_dir: self.download_dir.clone(),
            max_connections: self.max_connections,
            max_concurrent: self.max_concurrent,
            ffmpeg_path: path_of(ToolName::Ffmpeg),
            ytdlp_path: path_of(ToolName::Ytdlp),
            js_runtime: path_of(ToolName::Node),
        }
    }
}
