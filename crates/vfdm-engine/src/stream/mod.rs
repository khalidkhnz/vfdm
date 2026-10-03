//! HLS / DASH downloads: manifest → ordered segment list → parallel fetch →
//! in-order append into the `.part` → optional ffmpeg remux/mux.

pub mod detect;
pub mod hls;
pub mod mpd;

pub(crate) mod crypto;
pub(crate) mod fetch;
pub(crate) mod mux;
pub(crate) mod writer;

use crate::download::{self, RunCtx, RunResult};
use crate::error::{EngineError, Result};
use crate::filename;
use crate::types::*;
use crate::worker::Outcome;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub url: Url,
    /// Inclusive byte range within `url`.
    pub range: Option<(u64, u64)>,
}

#[derive(Debug, Clone)]
pub struct KeySpec {
    pub url: Url,
    pub iv: [u8; 16],
}

#[derive(Debug, Clone)]
pub struct SegmentPlan {
    pub res: Resource,
    pub key: Option<KeySpec>,
    pub init_idx: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Ts,
    Fmp4,
    Webm,
}

impl Container {
    pub fn ext(self, role: TrackRole) -> &'static str {
        match (self, role) {
            (Self::Ts, _) => "ts",
            (Self::Fmp4, TrackRole::Audio) => "m4a",
            (Self::Fmp4, _) => "mp4",
            (Self::Webm, _) => "webm",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrackPlan {
    pub role: TrackRole,
    pub inits: Vec<Resource>,
    pub segments: Vec<SegmentPlan>,
    pub container: Container,
    pub fingerprint: String,
}

impl TrackPlan {
    pub fn fingerprint_of(segments: &[SegmentPlan]) -> String {
        match (segments.first(), segments.last()) {
            (Some(f), Some(l)) => {
                format!(
                    "{}|{}|{}",
                    segments.len(),
                    f.res.url.path(),
                    l.res.url.path()
                )
            }
            _ => "0".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub manifest_url: Url,
    pub variants: Vec<VariantInfo>,
    pub chosen: Option<u32>,
    pub tracks: Vec<TrackPlan>,
}

impl Plan {
    /// Extension of the final (muxed) output.
    pub fn container_ext(&self) -> String {
        let t = &self.tracks[0];
        let role = if self.tracks.len() == 1 {
            t.role
        } else {
            TrackRole::Video
        };
        t.container.ext(role).to_string()
    }
}

pub async fn run(ctx: Arc<RunCtx>) -> RunResult {
    ctx.set_status(DownloadStatus::Probing);
    let plan = match prepare(&ctx).await {
        Ok(p) => p,
        Err(EngineError::Cancelled) => return RunResult::Interrupted,
        Err(e) => return RunResult::Failed(e.to_string()),
    };
    if ctx.cancel.is_cancelled() {
        return RunResult::Interrupted;
    }

    ctx.set_status(DownloadStatus::Downloading);
    let local = ctx.cancel.child_token();
    let keys = Arc::new(crypto::KeyCache::default());
    for (i, track) in plan.tracks.iter().enumerate() {
        let outcome = writer::run_track(
            ctx.clone(),
            local.clone(),
            i,
            Arc::new(track.clone()),
            keys.clone(),
        )
        .await;
        let _ = ctx.store.save(&ctx.lock_meta());
        match outcome {
            Outcome::Done => {}
            Outcome::Cancelled => return RunResult::Interrupted,
            Outcome::RemoteChanged => {
                return RunResult::Failed(EngineError::RemoteChanged.to_string())
            }
            Outcome::RangeIgnored => {
                return RunResult::Failed("server ignored a segment byte range".into())
            }
            Outcome::Failed(e) => return RunResult::Failed(e.to_string()),
        }
    }
    finalize(&ctx, &plan).await
}

/// Fresh: fetch + parse the manifest, pick a name, create empty parts.
/// Resume: re-fetch (signed URLs expire), compare fingerprints, trim torn tails.
async fn prepare(ctx: &RunCtx) -> Result<Plan> {
    let (fresh, request, kind, prior_chosen) = {
        let m = ctx.lock_meta();
        (
            m.stream.is_none(),
            m.request.clone(),
            m.kind,
            m.stream.as_ref().and_then(|s| s.chosen),
        )
    };
    let plan = match kind {
        Kind::Hls => hls::load(ctx, &request, prior_chosen).await?,
        Kind::Dash => mpd::load(ctx, &request).await?,
        other => {
            return Err(EngineError::Other(format!(
                "stream runner called for kind {other:?}"
            )))
        }
    };
    if plan.tracks.is_empty() || plan.tracks.iter().any(|t| t.segments.is_empty()) {
        return Err(EngineError::Other("playlist has no segments".into()));
    }
    let mut plan = plan;
    tokio::select! {
        _ = ctx.cancel.cancelled() => return Err(EngineError::Cancelled),
        r = expand_open_ranges(ctx, &request, &mut plan) => r?,
    }

    let mut m = ctx.lock_meta();
    if fresh {
        let dest = request
            .dest_dir
            .clone()
            .unwrap_or_else(|| ctx.default_dir.clone());
        std::fs::create_dir_all(&dest)?;
        let ext = plan.container_ext();
        let name =
            filename::stream_output_name(request.filename.as_deref(), &plan.manifest_url, &ext);
        m.final_path = filename::dedupe(&dest, &name);
        m.part_path = filename::part_path(&m.final_path);
        m.filename = file_name(&m.final_path);
        m.final_url = plan.manifest_url.to_string();
        m.resumable = true;
        m.total = None;
        m.segments.clear();
        let single = plan.tracks.len() == 1;
        let tracks: Vec<TrackState> = plan
            .tracks
            .iter()
            .map(|t| TrackState {
                role: t.role,
                part_path: if single {
                    m.part_path.clone()
                } else {
                    track_part_path(&m.final_path, t.role)
                },
                segment_count: t.segments.len() as u32,
                segments_done: 0,
                bytes_done: 0,
                last_init: None,
                fingerprint: t.fingerprint.clone(),
                ready: Vec::new(),
            })
            .collect();
        for t in &tracks {
            File::create(&t.part_path)?;
        }
        m.stream = Some(StreamState {
            manifest_url: plan.manifest_url.to_string(),
            variants: plan.variants.clone(),
            chosen: plan.chosen,
            tracks,
            container_ext: ext,
        });
    } else {
        let st = m.stream.as_mut().expect("stream state on resume");
        let same = st.tracks.len() == plan.tracks.len()
            && st
                .tracks
                .iter()
                .zip(&plan.tracks)
                .all(|(a, b)| a.fingerprint == b.fingerprint);
        if !same {
            return Err(EngineError::RemoteChanged);
        }
        st.manifest_url = plan.manifest_url.to_string();
        for t in &mut st.tracks {
            t.ready.clear();
            match std::fs::metadata(&t.part_path) {
                Ok(md) if md.len() >= t.bytes_done => {
                    if md.len() > t.bytes_done {
                        OpenOptions::new()
                            .write(true)
                            .open(&t.part_path)?
                            .set_len(t.bytes_done)?;
                    }
                }
                _ => {
                    t.segments_done = 0;
                    t.bytes_done = 0;
                    t.last_init = None;
                    File::create(&t.part_path)?;
                }
            }
        }
        m.final_url = plan.manifest_url.to_string();
    }
    ctx.store.save(&m)?;
    Ok(plan)
}

/// DASH SegmentBase representations are one open-ended range. Probe the size
/// and split into fixed chunks so progress and parallel fetching work.
async fn expand_open_ranges(
    ctx: &RunCtx,
    request: &DownloadRequest,
    plan: &mut Plan,
) -> Result<()> {
    const CHUNK: u64 = 4 << 20;
    for track in &mut plan.tracks {
        if !track
            .segments
            .iter()
            .any(|s| matches!(s.res.range, Some((_, e)) if e == u64::MAX))
        {
            continue;
        }
        let mut out = Vec::with_capacity(track.segments.len());
        for seg in track.segments.drain(..) {
            match seg.res.range {
                Some((start, end)) if end == u64::MAX => {
                    let probe_req = DownloadRequest {
                        url: seg.res.url.to_string(),
                        ..request.clone()
                    };
                    let pr = crate::probe::probe(&ctx.client, &probe_req).await?;
                    let total = pr.total.ok_or_else(|| {
                        EngineError::Other(
                            "cannot determine the size of a DASH representation".into(),
                        )
                    })?;
                    if start >= total {
                        return Err(EngineError::Other("DASH representation is empty".into()));
                    }
                    let mut s = start;
                    while s < total {
                        let e = (s + CHUNK - 1).min(total - 1);
                        out.push(SegmentPlan {
                            res: Resource {
                                url: seg.res.url.clone(),
                                range: Some((s, e)),
                            },
                            key: None,
                            init_idx: seg.init_idx,
                        });
                        s = e + 1;
                    }
                }
                _ => out.push(seg),
            }
        }
        track.segments = out;
        track.fingerprint = TrackPlan::fingerprint_of(&track.segments);
    }
    Ok(())
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn track_part_path(final_path: &Path, role: TrackRole) -> PathBuf {
    let mut s = final_path.as_os_str().to_owned();
    s.push(match role {
        TrackRole::Video => ".video.part",
        TrackRole::Audio => ".audio.part",
        TrackRole::Muxed => ".part",
    });
    PathBuf::from(s)
}

fn with_ext(path: &Path, ext: &str) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "stream".into());
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    filename::dedupe(dir, &format!("{stem}.{ext}"))
}

/// Remux/mux with ffmpeg when available, otherwise rename what we have.
async fn finalize(ctx: &RunCtx, plan: &Plan) -> RunResult {
    ctx.set_status(DownloadStatus::Processing);
    let (tracks, final_path, container_ext) = {
        let m = ctx.lock_meta();
        let st = m.stream.as_ref().expect("stream state");
        (
            st.tracks.clone(),
            m.final_path.clone(),
            st.container_ext.clone(),
        )
    };
    let ffmpeg = ctx.ffmpeg_path.clone();
    let mut note: Option<String> = None;

    let out: std::result::Result<PathBuf, String> = if tracks.len() == 1 {
        let part = &tracks[0].part_path;
        if container_ext == "ts" {
            match &ffmpeg {
                Some(ff) => {
                    let mp4 = with_ext(&final_path, "mp4");
                    let tmp = filename::part_path(&mp4);
                    match mux::remux_ts_to_mp4(ff, part, &tmp, &ctx.cancel).await {
                        Ok(()) => {
                            let _ = std::fs::remove_file(part);
                            download::rename_with_retry(&tmp, &mp4).await.map(|_| mp4)
                        }
                        Err(EngineError::Cancelled) => return RunResult::Interrupted,
                        Err(e) => {
                            let _ = std::fs::remove_file(&tmp);
                            note = Some(format!("ffmpeg remux failed ({e}); kept .ts"));
                            download::rename_with_retry(part, &final_path)
                                .await
                                .map(|_| final_path.clone())
                        }
                    }
                }
                None => {
                    note =
                        Some("Install ffmpeg (Settings → Tools) to get .mp4 instead of .ts".into());
                    download::rename_with_retry(part, &final_path)
                        .await
                        .map(|_| final_path.clone())
                }
            }
        } else {
            download::rename_with_retry(part, &final_path)
                .await
                .map(|_| final_path.clone())
        }
    } else {
        let video = tracks.iter().find(|t| t.role == TrackRole::Video);
        let audio = tracks.iter().find(|t| t.role == TrackRole::Audio);
        let (Some(v), Some(a)) = (video, audio) else {
            return RunResult::Failed("unexpected track layout".into());
        };
        let muxed = match &ffmpeg {
            Some(ff) => {
                let tmp = filename::part_path(&final_path);
                match mux::mux_av(ff, &v.part_path, &a.part_path, &tmp, &ctx.cancel).await {
                    Ok(()) => {
                        let _ = std::fs::remove_file(&v.part_path);
                        let _ = std::fs::remove_file(&a.part_path);
                        Some(
                            download::rename_with_retry(&tmp, &final_path)
                                .await
                                .map(|_| final_path.clone()),
                        )
                    }
                    Err(EngineError::Cancelled) => return RunResult::Interrupted,
                    Err(e) => {
                        let _ = std::fs::remove_file(&tmp);
                        note = Some(format!(
                            "ffmpeg mux failed ({e}); saved video and audio separately"
                        ));
                        None
                    }
                }
            }
            None => {
                note = Some("Saved video and audio separately — install ffmpeg (Settings → Tools) to mux them".into());
                None
            }
        };
        match muxed {
            Some(r) => r,
            None => {
                let vext = plan.tracks[0].container.ext(TrackRole::Video);
                let aext = plan
                    .tracks
                    .iter()
                    .find(|t| t.role == TrackRole::Audio)
                    .map(|t| t.container.ext(TrackRole::Audio))
                    .unwrap_or("m4a");
                let stem = final_path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "stream".into());
                let dir = final_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf();
                let vout = filename::dedupe(&dir, &format!("{stem}.video.{vext}"));
                let aout = filename::dedupe(&dir, &format!("{stem}.audio.{aext}"));
                match download::rename_with_retry(&v.part_path, &vout).await {
                    Ok(()) => download::rename_with_retry(&a.part_path, &aout)
                        .await
                        .map(|_| vout),
                    Err(e) => Err(e),
                }
            }
        }
    };

    match out {
        Ok(path) => {
            {
                let mut m = ctx.lock_meta();
                m.filename = file_name(&path);
                m.final_path = path;
                m.note = note;
                let (total, _) = m.effective_total();
                m.total = Some(m.downloaded()).or(total);
                m.total_is_estimate = false;
            }
            download::mark_completed(ctx);
            RunResult::Completed
        }
        Err(e) => RunResult::Failed(e),
    }
}
