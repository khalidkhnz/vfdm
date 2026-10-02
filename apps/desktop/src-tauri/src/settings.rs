use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use vfdm_engine::Settings as EngineSettings;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub download_dir: PathBuf,
    pub max_connections: u8,
    pub max_concurrent: u8,
    pub notify_on_complete: bool,
    pub focus_on_capture: bool,
}

impl AppSettings {
    fn defaults(download_dir: PathBuf) -> Self {
        Self {
            download_dir,
            max_connections: 8,
            max_concurrent: 3,
            notify_on_complete: true,
            focus_on_capture: true,
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
        self
    }

    pub fn to_engine(&self) -> EngineSettings {
        EngineSettings {
            download_dir: self.download_dir.clone(),
            max_connections: self.max_connections,
            max_concurrent: self.max_concurrent,
            ffmpeg_path: None,
            ytdlp_path: None,
            js_runtime: None,
        }
    }
}
