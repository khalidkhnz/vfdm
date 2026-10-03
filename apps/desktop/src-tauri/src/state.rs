use crate::bridge::{self, BridgeInfo};
use crate::events::EV_TOOLS;
use crate::settings::AppSettings;
use crate::tools::{self, ToolStatus};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Emitter, Manager};
use vfdm_engine::Engine;

#[derive(Clone)]
pub struct AppState {
    pub app: AppHandle,
    pub engine: Engine,
    pub data_dir: PathBuf,
    pub settings: Arc<RwLock<AppSettings>>,
    pub bridge: Arc<RwLock<BridgeInfo>>,
    pub tools: Arc<RwLock<Vec<ToolStatus>>>,
}

impl AppState {
    pub async fn init(app: &AppHandle) -> anyhow::Result<Self> {
        let data_dir = app.path().app_data_dir()?;
        std::fs::create_dir_all(&data_dir)?;
        let default_dl = app
            .path()
            .download_dir()
            .unwrap_or_else(|_| data_dir.join("downloads"));
        let settings = AppSettings::load(&data_dir, default_dl);
        let tools = tools::resolve_all(
            &data_dir,
            settings.ffmpeg_path.as_deref(),
            settings.ytdlp_path.as_deref(),
        )
        .await;
        let engine = Engine::new(settings.to_engine(&tools), &data_dir)?;
        let n = engine.load_all()?;
        tracing::info!(count = n, dir = %data_dir.display(), "engine ready");
        for t in &tools {
            tracing::info!(tool = ?t.name, path = ?t.path, version = ?t.version, "tool");
        }
        let token = bridge::load_or_create_token(&data_dir)?;
        Ok(Self {
            app: app.clone(),
            engine,
            data_dir,
            settings: Arc::new(RwLock::new(settings)),
            bridge: Arc::new(RwLock::new(BridgeInfo { port: 0, token })),
            tools: Arc::new(RwLock::new(tools)),
        })
    }

    pub fn settings(&self) -> AppSettings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn tool_status(&self) -> Vec<ToolStatus> {
        self.tools.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Re-detect binaries and push the resolved paths into the engine.
    pub async fn refresh_tools(&self) {
        let s = self.settings();
        let tools = tools::resolve_all(
            &self.data_dir,
            s.ffmpeg_path.as_deref(),
            s.ytdlp_path.as_deref(),
        )
        .await;
        self.engine.set_settings(s.to_engine(&tools));
        *self.tools.write().unwrap_or_else(|e| e.into_inner()) = tools.clone();
        let _ = self.app.emit(EV_TOOLS, &tools);
    }
}

mod anyhow {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
}
