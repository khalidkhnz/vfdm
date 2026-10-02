use crate::bridge::{self, BridgeInfo};
use crate::settings::AppSettings;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Manager};
use vfdm_engine::Engine;

#[derive(Clone)]
pub struct AppState {
    pub engine: Engine,
    pub data_dir: PathBuf,
    pub settings: Arc<RwLock<AppSettings>>,
    pub bridge: Arc<RwLock<BridgeInfo>>,
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
        let engine = Engine::new(settings.to_engine(), &data_dir)?;
        let n = engine.load_all()?;
        tracing::info!(count = n, dir = %data_dir.display(), "engine ready");
        let token = bridge::load_or_create_token(&data_dir)?;
        Ok(Self {
            engine,
            data_dir,
            settings: Arc::new(RwLock::new(settings)),
            bridge: Arc::new(RwLock::new(BridgeInfo { port: 0, token })),
        })
    }

    pub fn settings(&self) -> AppSettings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

mod anyhow {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
}
