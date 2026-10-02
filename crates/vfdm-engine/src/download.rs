use crate::error::{EngineError, Result};
use crate::filename;
use crate::planner;
use crate::probe::probe;
use crate::state::Store;
use crate::types::*;
use crate::worker::{self, Outcome};
use crate::writer::preallocate;
use reqwest::Client;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::{broadcast, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

pub struct RunCtx {
    pub id: DownloadId,
    pub meta: Arc<Mutex<DownloadMeta>>,
    pub client: Client,
    pub store: Store,
    pub cancel: CancellationToken,
    pub host_sem: Arc<Semaphore>,
    pub max_connections: u8,
    pub default_dir: PathBuf,
    pub events: broadcast::Sender<DownloadEvent>,
}

impl RunCtx {
    pub fn lock_meta(&self) -> MutexGuard<'_, DownloadMeta> {
        self.meta.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_status(&self, status: DownloadStatus) {
        self.lock_meta().status = status.clone();
        let _ = self.events.send(DownloadEvent::Status {
            id: self.id,
            status,
        });
    }
}

#[derive(Debug)]
pub enum RunResult {
    Completed,
    Failed(String),
    /// Cancelled via the token; the scheduler decides Paused vs Cancelled.
    Interrupted,
}

pub async fn run(ctx: Arc<RunCtx>) -> RunResult {
    ctx.set_status(DownloadStatus::Probing);
    match prepare(&ctx).await {
        Ok(()) => {}
        Err(EngineError::Cancelled) => return RunResult::Interrupted,
        Err(e) => return RunResult::Failed(e.to_string()),
    }
    if ctx.cancel.is_cancelled() {
        return RunResult::Interrupted;
    }

    let (resumable, total) = {
        let m = ctx.lock_meta();
        (m.resumable, m.total)
    };
    if total == Some(0) {
        let part = ctx.lock_meta().part_path.clone();
        if let Err(e) = preallocate(&part, 0) {
            return RunResult::Failed(e.to_string());
        }
        return finalize(&ctx).await;
    }

    ctx.set_status(DownloadStatus::Downloading);
    let local = ctx.cancel.child_token();
    let mut outcome = if resumable {
        pool(ctx.clone(), local.clone()).await
    } else {
        worker::run_single(ctx.clone(), local.clone()).await
    };

    if matches!(outcome, Outcome::RangeIgnored) {
        tracing::warn!(
            id = ctx.id,
            "server ignored Range; restarting as single stream"
        );
        {
            let mut m = ctx.lock_meta();
            m.resumable = false;
            m.segments = vec![single_segment(m.total)];
        }
        outcome = worker::run_single(ctx.clone(), local).await;
    }

    let _ = ctx.store.save(&ctx.lock_meta());
    match outcome {
        Outcome::Done => finalize(&ctx).await,
        Outcome::Cancelled => RunResult::Interrupted,
        Outcome::RemoteChanged => RunResult::Failed(EngineError::RemoteChanged.to_string()),
        Outcome::RangeIgnored => RunResult::Failed("server ignored range request twice".into()),
        Outcome::Failed(e) => RunResult::Failed(e.to_string()),
    }
}

fn single_segment(total: Option<u64>) -> Segment {
    Segment {
        id: 0,
        start: 0,
        end: total.map(|t| t.saturating_sub(1)).unwrap_or(u64::MAX),
        downloaded: 0,
    }
}

/// Fresh download: probe, pick a filename, plan segments, preallocate.
/// Resume: re-probe the original URL (signed URLs expire), verify validators.
async fn prepare(ctx: &RunCtx) -> Result<()> {
    let (fresh, request) = {
        let m = ctx.lock_meta();
        (m.final_path.as_os_str().is_empty(), m.request.clone())
    };
    let pr = tokio::select! {
        _ = ctx.cancel.cancelled() => return Err(EngineError::Cancelled),
        r = probe(&ctx.client, &request) => r?,
    };

    let mut m = ctx.lock_meta();
    if fresh {
        let dest = request
            .dest_dir
            .clone()
            .unwrap_or_else(|| ctx.default_dir.clone());
        std::fs::create_dir_all(&dest)?;
        m.final_path = filename::dedupe(&dest, &pr.filename);
        m.part_path = filename::part_path(&m.final_path);
        m.filename = m
            .final_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or(pr.filename.clone());
        m.final_url = pr.final_url.to_string();
        m.total = pr.total;
        m.etag = pr.etag.clone();
        m.last_modified = pr.last_modified.clone();
        m.resumable = pr.accepts_ranges && pr.total.is_some_and(|t| t > 0);
        if m.resumable {
            let total = m.total.unwrap_or(0);
            m.segments = planner::initial_split(total, ctx.max_connections);
            preallocate(&m.part_path, total)?;
        } else {
            m.segments = vec![single_segment(m.total)];
        }
    } else if m.resumable {
        let etag_changed = m.etag.is_some() && pr.etag != m.etag;
        let lm_changed =
            m.etag.is_none() && m.last_modified.is_some() && pr.last_modified != m.last_modified;
        if etag_changed || lm_changed || pr.total != m.total || !pr.accepts_ranges {
            return Err(EngineError::RemoteChanged);
        }
        m.final_url = pr.final_url.to_string();
        if !m.part_path.exists() {
            for s in &mut m.segments {
                s.downloaded = 0;
            }
            preallocate(&m.part_path, m.total.unwrap_or(0))?;
        }
    } else {
        m.final_url = pr.final_url.to_string();
        m.total = pr.total;
        m.segments = vec![single_segment(m.total)];
    }
    ctx.store.save(&m)?;
    Ok(())
}

/// Keeps `max_connections` workers busy; when a worker finishes and nothing
/// is unassigned, it steals the back half of the largest remaining segment.
async fn pool(ctx: Arc<RunCtx>, cancel: CancellationToken) -> Outcome {
    let max = ctx.max_connections.max(1) as usize;
    let mut set: JoinSet<(u32, Outcome)> = JoinSet::new();
    let mut assigned: Vec<u32> = Vec::new();

    loop {
        while set.len() < max {
            let next_id = {
                let mut m = ctx.lock_meta();
                let unassigned = planner::unassigned(&m.segments, &assigned)
                    .next()
                    .map(|s| s.id);
                unassigned.or_else(|| planner::steal(&mut m.segments).map(|s| s.id))
            };
            let Some(id) = next_id else { break };
            assigned.push(id);
            let c = ctx.clone();
            let tok = cancel.clone();
            set.spawn(async move { (id, worker::run_segment(c, tok, id).await) });
        }
        if set.is_empty() {
            break;
        }
        let (id, outcome) = match set.join_next().await {
            Some(Ok(r)) => r,
            Some(Err(e)) => (
                u32::MAX,
                Outcome::Failed(EngineError::Other(format!("worker panicked: {e}"))),
            ),
            None => break,
        };
        assigned.retain(|&a| a != id);
        match outcome {
            Outcome::Done => {}
            other => {
                cancel.cancel();
                while set.join_next().await.is_some() {}
                return other;
            }
        }
    }

    if ctx.lock_meta().all_done() {
        Outcome::Done
    } else {
        Outcome::Failed(EngineError::Other("segments incomplete".into()))
    }
}

async fn finalize(ctx: &RunCtx) -> RunResult {
    let (part, fin) = {
        let m = ctx.lock_meta();
        (m.part_path.clone(), m.final_path.clone())
    };
    // Windows refuses the rename while AV scanners hold the handle; retry briefly.
    let mut last = None;
    for _ in 0..5 {
        match std::fs::rename(&part, &fin) {
            Ok(()) => {
                last = None;
                break;
            }
            Err(e) => {
                last = Some(e);
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
    if let Some(e) = last {
        return RunResult::Failed(format!("rename failed: {e}"));
    }
    {
        let mut m = ctx.lock_meta();
        m.status = DownloadStatus::Completed;
        m.completed_at = Some(now_millis());
        m.request.cookies = None;
        m.request.headers.clear();
        let _ = ctx.store.save(&m);
    }
    RunResult::Completed
}
