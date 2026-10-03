use crate::state::AppState;
use std::collections::HashMap;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::{self, error::RecvError};
use vfdm_engine::{DownloadEvent, DownloadStatus, Progress};

pub const EV_PROGRESS: &str = "download://progress";
pub const EV_STATUS: &str = "download://status";
pub const EV_ADDED: &str = "download://added";
pub const EV_REMOVED: &str = "download://removed";
pub const EV_BRIDGE_REQUEST: &str = "bridge://request";
pub const EV_TOOLS: &str = "tools://changed";

const BATCH_INTERVAL: Duration = Duration::from_millis(100);

/// Engine events arrive at up to 4Hz per download; the UI gets one batched
/// `download://progress` array every 100ms plus immediate status changes.
pub fn spawn_forwarder(app: AppHandle, mut rx: broadcast::Receiver<DownloadEvent>) {
    tauri::async_runtime::spawn(async move {
        let mut pending: HashMap<u64, Progress> = HashMap::new();
        let mut tick = tokio::time::interval(BATCH_INTERVAL);
        loop {
            tokio::select! {
                ev = rx.recv() => match ev {
                    Ok(DownloadEvent::Progress(p)) => { pending.insert(p.id, p); }
                    Ok(DownloadEvent::Added(p)) => { let _ = app.emit(EV_ADDED, &p); }
                    Ok(DownloadEvent::Removed { id }) => { pending.remove(&id); let _ = app.emit(EV_REMOVED, id); }
                    Ok(DownloadEvent::Status { id, status }) => {
                        if let Some(p) = app.state::<AppState>().engine.get(id) {
                            pending.insert(id, p.clone());
                            if status == DownloadStatus::Completed {
                                notify_complete(&app, &p);
                            }
                        }
                        let _ = app.emit(EV_STATUS, serde_json::json!({ "id": id, "status": status }));
                    }
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                },
                _ = tick.tick() => {
                    if !pending.is_empty() {
                        let batch: Vec<Progress> = pending.drain().map(|(_, p)| p).collect();
                        let _ = app.emit(EV_PROGRESS, &batch);
                    }
                }
            }
        }
    });
}

fn notify_complete(app: &AppHandle, p: &Progress) {
    if !app.state::<AppState>().settings().notify_on_complete {
        return;
    }
    let _ = app
        .notification()
        .builder()
        .title("Download complete")
        .body(&p.filename)
        .show();
}
