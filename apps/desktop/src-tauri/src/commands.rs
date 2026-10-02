use crate::bridge::{self, BridgeInfo};
use crate::settings::AppSettings;
use crate::state::AppState;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use vfdm_engine::{DownloadId, DownloadRequest, Progress};

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub fn list_downloads(state: State<'_, AppState>) -> Vec<Progress> {
    state.engine.list()
}

#[tauri::command]
pub fn add_download(state: State<'_, AppState>, req: DownloadRequest) -> CmdResult<DownloadId> {
    state.engine.add(req).map_err(err)
}

#[tauri::command]
pub fn pause_download(state: State<'_, AppState>, id: DownloadId) -> CmdResult<()> {
    state.engine.pause(id).map_err(err)
}

#[tauri::command]
pub fn resume_download(state: State<'_, AppState>, id: DownloadId) -> CmdResult<()> {
    state.engine.resume(id).map_err(err)
}

#[tauri::command]
pub fn retry_download(state: State<'_, AppState>, id: DownloadId) -> CmdResult<()> {
    state.engine.retry(id).map_err(err)
}

#[tauri::command]
pub fn cancel_download(
    state: State<'_, AppState>,
    id: DownloadId,
    delete_files: bool,
) -> CmdResult<()> {
    state.engine.cancel(id, delete_files).map_err(err)
}

#[tauri::command]
pub async fn remove_download(
    state: State<'_, AppState>,
    id: DownloadId,
    delete_files: bool,
) -> CmdResult<()> {
    state.engine.remove(id, delete_files).await.map_err(err)
}

#[tauri::command]
pub fn open_file(state: State<'_, AppState>, id: DownloadId) -> CmdResult<()> {
    let p = state.engine.get(id).ok_or("not found")?;
    tauri_plugin_opener::open_path(p.final_path, None::<&str>).map_err(err)
}

#[tauri::command]
pub fn reveal_file(state: State<'_, AppState>, id: DownloadId) -> CmdResult<()> {
    let p = state.engine.get(id).ok_or("not found")?;
    let path = if p.final_path.exists() {
        p.final_path
    } else {
        vfdm_engine::filename::part_path(&p.final_path)
    };
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(err)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppSettings {
    state.settings()
}

#[tauri::command]
pub fn set_settings(state: State<'_, AppState>, settings: AppSettings) -> CmdResult<AppSettings> {
    let settings = settings.clamped();
    settings.save(&state.data_dir).map_err(err)?;
    state.engine.set_settings(settings.to_engine());
    *state.settings.write().unwrap_or_else(|e| e.into_inner()) = settings.clone();
    Ok(settings)
}

#[tauri::command]
pub fn get_bridge_info(state: State<'_, AppState>) -> BridgeInfo {
    state
        .bridge
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

#[tauri::command]
pub fn regenerate_token(state: State<'_, AppState>) -> CmdResult<BridgeInfo> {
    let token = bridge::write_new_token(&state.data_dir).map_err(err)?;
    let mut b = state.bridge.write().unwrap_or_else(|e| e.into_inner());
    b.token = token;
    Ok(b.clone())
}

#[tauri::command]
pub async fn pick_download_dir(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<String>> {
    let start = state.settings().download_dir;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_directory(start)
            .blocking_pick_folder()
    })
    .await
    .map_err(err)?;
    Ok(picked.map(|p| p.to_string()))
}
