use crate::download::{self, RunCtx, RunResult};
use crate::error::{EngineError, Result};
use crate::http::build_client;
use crate::planner;
use crate::speed::SpeedMeter;
use crate::state::Store;
use crate::types::*;
use crate::{stream, ytdlp};
use reqwest::Client;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};
use std::time::Duration;
use tokio::sync::{broadcast, Notify, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use url::Url;

const TICK: Duration = Duration::from_millis(250);
const SAVE_EVERY_TICKS: u64 = 4;
const DEDUPE_WINDOW_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    None,
    Pause,
    Cancel { delete_files: bool },
}

struct Entry {
    meta: Arc<Mutex<DownloadMeta>>,
    speed: SpeedMeter,
    cancel: Option<CancellationToken>,
    task: Option<JoinHandle<()>>,
    intent: Intent,
}

struct Inner {
    client: Client,
    store: Store,
    data_dir: PathBuf,
    settings: RwLock<Settings>,
    downloads: Mutex<BTreeMap<DownloadId, Entry>>,
    events: broadcast::Sender<DownloadEvent>,
    next_id: AtomicU64,
    hosts: Mutex<HashMap<String, Arc<Semaphore>>>,
    wake: Notify,
    bg: Mutex<Vec<JoinHandle<()>>>,
}

/// Queue + lifecycle owner. Cheap to clone. Must be created inside a Tokio runtime.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

impl Engine {
    pub fn new(settings: Settings, data_dir: &Path) -> Result<Self> {
        Self::with_client(settings, data_dir, build_client())
    }

    pub fn with_client(settings: Settings, data_dir: &Path, client: Client) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let (events, _) = broadcast::channel(1024);
        let inner = Arc::new(Inner {
            client,
            store: Store::new(data_dir)?,
            data_dir: data_dir.to_path_buf(),
            settings: RwLock::new(settings),
            downloads: Mutex::new(BTreeMap::new()),
            events,
            next_id: AtomicU64::new(1),
            hosts: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            bg: Mutex::new(Vec::new()),
        });
        let pump = tokio::spawn(pump(Arc::downgrade(&inner)));
        let ticker = tokio::spawn(ticker(Arc::downgrade(&inner)));
        lock(&inner.bg).extend([pump, ticker]);
        Ok(Self { inner })
    }

    /// Reload sidecars. Anything that was mid-flight when the process died comes back Paused.
    pub fn load_all(&self) -> Result<usize> {
        let metas = self.inner.store.load_all()?;
        let mut map = lock(&self.inner.downloads);
        let mut max_id = 0;
        for mut m in metas {
            max_id = max_id.max(m.id);
            let mut dirty = false;
            // v1 rows were always plain files; leaving them Auto would re-probe on resume.
            if m.version < 2 {
                m.kind = Kind::File;
                m.version = META_VERSION;
                dirty = true;
            }
            if m.status.is_active() {
                m.status = DownloadStatus::Paused;
                dirty = true;
            }
            if dirty {
                let _ = self.inner.store.save(&m);
            }
            let downloaded = m.downloaded();
            map.insert(m.id, Entry::new(m, downloaded));
        }
        self.inner.next_id.fetch_max(max_id + 1, Ordering::SeqCst);
        let n = map.len();
        drop(map);
        self.inner.wake.notify_one();
        Ok(n)
    }

    pub fn add(&self, request: DownloadRequest) -> Result<DownloadId> {
        Url::parse(&request.url).map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
        let now = now_millis();
        let mut map = lock(&self.inner.downloads);

        let dup = map.values().find_map(|e| {
            let m = lock(&e.meta);
            let recent = now.saturating_sub(m.created_at) < DEDUPE_WINDOW_MS;
            (recent && !m.status.is_terminal() && m.request.url == request.url).then_some(m.id)
        });
        if let Some(id) = dup {
            return Ok(id);
        }

        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let filename = request
            .filename
            .clone()
            .or_else(|| {
                Url::parse(&request.url)
                    .ok()
                    .and_then(|u| crate::filename::from_url(&u))
            })
            .unwrap_or_else(|| "download".into());
        let kind = request.kind;
        let meta = DownloadMeta {
            version: META_VERSION,
            id,
            kind,
            total_is_estimate: false,
            note: None,
            stream: None,
            request,
            final_url: String::new(),
            filename,
            part_path: Default::default(),
            final_path: Default::default(),
            total: None,
            etag: None,
            last_modified: None,
            resumable: false,
            segments: Vec::new(),
            status: DownloadStatus::Queued,
            created_at: now,
            completed_at: None,
        };
        self.inner.store.save(&meta)?;
        let progress = to_progress(&meta, 0.0, None);
        map.insert(id, Entry::new(meta, 0));
        drop(map);
        let _ = self.inner.events.send(DownloadEvent::Added(progress));
        self.inner.wake.notify_one();
        Ok(id)
    }

    pub fn pause(&self, id: DownloadId) -> Result<()> {
        let mut map = lock(&self.inner.downloads);
        let e = map.get_mut(&id).ok_or(EngineError::NotFound(id))?;
        let status = lock(&e.meta).status.clone();
        match status {
            DownloadStatus::Queued => {
                self.inner.set_status(e, DownloadStatus::Paused);
                Ok(())
            }
            s if s.is_active() => {
                e.intent = Intent::Pause;
                if let Some(t) = &e.cancel {
                    t.cancel();
                }
                Ok(())
            }
            _ => Err(EngineError::BadState(id, "not running")),
        }
    }

    pub fn resume(&self, id: DownloadId) -> Result<()> {
        let mut map = lock(&self.inner.downloads);
        let e = map.get_mut(&id).ok_or(EngineError::NotFound(id))?;
        let status = lock(&e.meta).status.clone();
        match status {
            DownloadStatus::Paused | DownloadStatus::Failed(_) | DownloadStatus::Cancelled => {
                e.intent = Intent::None;
                self.inner.set_status(e, DownloadStatus::Queued);
                drop(map);
                self.inner.wake.notify_one();
                Ok(())
            }
            _ => Err(EngineError::BadState(id, "not paused")),
        }
    }

    pub fn retry(&self, id: DownloadId) -> Result<()> {
        self.resume(id)
    }

    pub fn cancel(&self, id: DownloadId, delete_files: bool) -> Result<()> {
        let mut map = lock(&self.inner.downloads);
        let e = map.get_mut(&id).ok_or(EngineError::NotFound(id))?;
        let status = lock(&e.meta).status.clone();
        if status.is_active() {
            e.intent = Intent::Cancel { delete_files };
            if let Some(t) = &e.cancel {
                t.cancel();
            }
            return Ok(());
        }
        if status.is_terminal() {
            return Err(EngineError::BadState(id, "already finished"));
        }
        self.inner.set_status(e, DownloadStatus::Cancelled);
        if delete_files {
            for p in lock(&e.meta).partial_paths() {
                remove_path(&p);
            }
        }
        Ok(())
    }

    /// Stops the download if running, deletes its sidecar, and optionally its partial file.
    pub async fn remove(&self, id: DownloadId, delete_files: bool) -> Result<()> {
        let (task, parts, fin, status) = {
            let mut map = lock(&self.inner.downloads);
            let e = map.get_mut(&id).ok_or(EngineError::NotFound(id))?;
            e.intent = Intent::Cancel {
                delete_files: false,
            };
            if let Some(t) = &e.cancel {
                t.cancel();
            }
            let m = lock(&e.meta);
            (
                e.task.take(),
                m.partial_paths(),
                m.final_path.clone(),
                m.status.clone(),
            )
        };
        if let Some(t) = task {
            let _ = t.await;
        }
        lock(&self.inner.downloads).remove(&id);
        self.inner.store.delete(id)?;
        if delete_files {
            for p in parts {
                remove_path(&p);
            }
            if status != DownloadStatus::Completed {
                remove_path(&fin);
            }
        }
        let _ = self.inner.events.send(DownloadEvent::Removed { id });
        Ok(())
    }

    pub fn list(&self) -> Vec<Progress> {
        lock(&self.inner.downloads)
            .values()
            .map(|e| {
                let m = lock(&e.meta);
                let (total, _) = m.effective_total();
                let remaining = total.map(|t| t.saturating_sub(m.downloaded()));
                let eta = if m.status.is_active() {
                    remaining.and_then(|r| e.speed.eta_secs(r))
                } else {
                    None
                };
                let rate = if m.status.is_active() {
                    e.speed.rate()
                } else {
                    0.0
                };
                to_progress(&m, rate, eta)
            })
            .collect()
    }

    pub fn get(&self, id: DownloadId) -> Option<Progress> {
        self.list().into_iter().find(|p| p.id == id)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DownloadEvent> {
        self.inner.events.subscribe()
    }

    pub fn settings(&self) -> Settings {
        self.inner
            .settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set_settings(&self, s: Settings) {
        *self
            .inner
            .settings
            .write()
            .unwrap_or_else(|e| e.into_inner()) = s;
        self.inner.wake.notify_one();
    }

    /// Pauses everything that is running, waits for workers to exit, flushes sidecars,
    /// and stops the background tasks.
    pub async fn shutdown(&self) {
        let tasks: Vec<JoinHandle<()>> = {
            let mut map = lock(&self.inner.downloads);
            map.values_mut()
                .filter_map(|e| {
                    if lock(&e.meta).status.is_active() {
                        e.intent = Intent::Pause;
                        if let Some(t) = &e.cancel {
                            t.cancel();
                        }
                        e.task.take()
                    } else {
                        None
                    }
                })
                .collect()
        };
        for t in tasks {
            let _ = t.await;
        }
        for e in lock(&self.inner.downloads).values() {
            let _ = self.inner.store.save(&lock(&e.meta));
        }
        for h in lock(&self.inner.bg).drain(..) {
            h.abort();
        }
    }
}

impl Entry {
    fn new(meta: DownloadMeta, downloaded: u64) -> Self {
        Self {
            meta: Arc::new(Mutex::new(meta)),
            speed: SpeedMeter::new(downloaded),
            cancel: None,
            task: None,
            intent: Intent::None,
        }
    }
}

impl Inner {
    fn set_status(&self, e: &Entry, status: DownloadStatus) {
        let id = {
            let mut m = lock(&e.meta);
            m.status = status.clone();
            let _ = self.store.save(&m);
            m.id
        };
        let _ = self.events.send(DownloadEvent::Status { id, status });
    }

    fn host_sem(&self, url: &str, permits: u8) -> Arc<Semaphore> {
        let key = Url::parse(url)
            .ok()
            .map(|u| {
                format!(
                    "{}://{}:{}",
                    u.scheme(),
                    u.host_str().unwrap_or(""),
                    u.port_or_known_default().unwrap_or(0)
                )
            })
            .unwrap_or_else(|| url.to_string());
        lock(&self.hosts)
            .entry(key)
            .or_insert_with(|| Arc::new(Semaphore::new(permits.max(1) as usize)))
            .clone()
    }

    /// Fill free slots with the oldest queued downloads.
    fn schedule(self: &Arc<Self>) {
        let settings = self
            .settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut map = lock(&self.downloads);
        let mut active = map
            .values()
            .filter(|e| lock(&e.meta).status.is_active())
            .count();

        let mut queued: Vec<(u64, DownloadId)> = map
            .values()
            .filter_map(|e| {
                let m = lock(&e.meta);
                (m.status == DownloadStatus::Queued).then_some((m.created_at, m.id))
            })
            .collect();
        queued.sort();

        for (_, id) in queued {
            if active >= settings.max_concurrent.max(1) as usize {
                break;
            }
            let Some(e) = map.get_mut(&id) else { continue };
            let (url, per_download, kind) = {
                let mut m = lock(&e.meta);
                m.status = DownloadStatus::Probing;
                (m.request.url.clone(), m.request.max_connections, m.kind)
            };
            let max_connections =
                planner::clamp_connections(per_download.unwrap_or(settings.max_connections));
            let cancel = CancellationToken::new();
            let ctx = Arc::new(RunCtx {
                id,
                meta: e.meta.clone(),
                client: self.client.clone(),
                store: self.store.clone(),
                cancel: cancel.clone(),
                host_sem: self.host_sem(&url, settings.max_connections),
                max_connections,
                default_dir: settings.download_dir.clone(),
                data_dir: self.data_dir.clone(),
                ffmpeg_path: settings.ffmpeg_path.clone(),
                ytdlp_path: settings.ytdlp_path.clone(),
                js_runtime: settings.js_runtime.clone(),
                events: self.events.clone(),
            });
            e.speed.reset(lock(&e.meta).downloaded());
            e.cancel = Some(cancel);
            e.intent = Intent::None;
            let inner = self.clone();
            e.task = Some(tokio::spawn(async move {
                let result = match kind {
                    Kind::Ytdlp => ytdlp::run(ctx).await,
                    Kind::Hls | Kind::Dash => stream::run(ctx).await,
                    Kind::Auto | Kind::File => download::run(ctx).await,
                };
                inner.on_finished(id, result);
            }));
            active += 1;
        }
    }

    fn on_finished(&self, id: DownloadId, result: RunResult) {
        let mut map = lock(&self.downloads);
        let Some(e) = map.get_mut(&id) else { return };
        e.cancel = None;
        let intent = std::mem::replace(&mut e.intent, Intent::None);
        let status = match result {
            RunResult::Completed => DownloadStatus::Completed,
            RunResult::Failed(msg) => DownloadStatus::Failed(msg),
            RunResult::Interrupted => match intent {
                Intent::Cancel { delete_files } => {
                    if delete_files {
                        for p in lock(&e.meta).partial_paths() {
                            remove_path(&p);
                        }
                    }
                    DownloadStatus::Cancelled
                }
                _ => DownloadStatus::Paused,
            },
        };
        self.set_status(e, status);
        drop(map);
        self.wake.notify_one();
    }
}

async fn pump(weak: Weak<Inner>) {
    loop {
        let Some(inner) = weak.upgrade() else { return };
        inner.schedule();
        let wake = &inner.wake;
        tokio::select! {
            _ = wake.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
    }
}

async fn ticker(weak: Weak<Inner>) {
    let mut n: u64 = 0;
    loop {
        tokio::time::sleep(TICK).await;
        n += 1;
        let Some(inner) = weak.upgrade() else { return };
        let mut map = lock(&inner.downloads);
        for e in map.values_mut() {
            let m = lock(&e.meta);
            if !m.status.is_active() {
                continue;
            }
            let downloaded = m.downloaded();
            let rate = e.speed.sample(downloaded);
            let (total, _) = m.effective_total();
            let eta = total.and_then(|t| e.speed.eta_secs(t.saturating_sub(downloaded)));
            let _ = inner
                .events
                .send(DownloadEvent::Progress(to_progress(&m, rate, eta)));
            if n.is_multiple_of(SAVE_EVERY_TICKS) {
                let _ = inner.store.save(&m);
            }
        }
    }
}

fn to_progress(m: &DownloadMeta, speed_bps: f64, eta_secs: Option<u64>) -> Progress {
    let (total, total_is_estimate) = m.effective_total();
    let (segments, segment_count, segments_done) = match (&m.stream, m.kind.is_stream()) {
        // Stream bars are in segment units: a contiguous written front per
        // track plus one dot per fetched-but-unwritten segment.
        (Some(st), true) => {
            let mut segs = Vec::new();
            let mut base = 0u64;
            for t in &st.tracks {
                let count = t.segment_count as u64;
                if t.segments_done > 0 {
                    segs.push(SegmentProgress {
                        start: base,
                        end: base + count.saturating_sub(1),
                        downloaded: t.segments_done as u64,
                    });
                }
                for &r in &t.ready {
                    segs.push(SegmentProgress {
                        start: base + r as u64,
                        end: base + r as u64,
                        downloaded: 1,
                    });
                }
                base += count;
            }
            let (count, done) = m.segment_counts().unwrap_or((0, 0));
            (segs, Some(count), Some(done))
        }
        _ => (
            m.segments
                .iter()
                .map(|s| SegmentProgress {
                    start: s.start,
                    end: s.end,
                    downloaded: s.downloaded,
                })
                .collect(),
            None,
            None,
        ),
    };
    Progress {
        id: m.id,
        status: m.status.clone(),
        kind: m.kind,
        url: m.request.url.clone(),
        filename: m.filename.clone(),
        final_path: m.final_path.clone(),
        total,
        total_is_estimate,
        downloaded: m.downloaded(),
        speed_bps,
        eta_secs,
        resumable: m.resumable,
        segments,
        segment_count,
        segments_done,
        note: m.note.clone(),
        created_at: m.created_at,
        completed_at: m.completed_at,
    }
}

/// Removes a partial file, or a whole temp directory (yt-dlp jobs).
fn remove_path(p: &Path) {
    if p.as_os_str().is_empty() {
        return;
    }
    if p.is_dir() {
        let _ = std::fs::remove_dir_all(p);
    } else {
        let _ = std::fs::remove_file(p);
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
