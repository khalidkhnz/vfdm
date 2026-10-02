use super::crypto::{self, KeyCache};
use super::{fetch, TrackPlan};
use crate::download::RunCtx;
use crate::error::{EngineError, Result};
use crate::types::DownloadRequest;
use crate::worker::Outcome;
use bytes::Bytes;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

const BUFFER_CAP: usize = 128 << 20;

/// Fetches a track's segments with bounded parallelism and appends them to
/// the `.part` strictly in order. Resume continues at `segments_done`.
pub async fn run_track(
    ctx: Arc<RunCtx>,
    cancel: CancellationToken,
    track_idx: usize,
    plan: Arc<TrackPlan>,
    keys: Arc<KeyCache>,
) -> Outcome {
    let (request, part_path, mut next_write, mut last_init) = {
        let m = ctx.lock_meta();
        let t = &m.stream.as_ref().expect("stream state").tracks[track_idx];
        (
            m.request.clone(),
            t.part_path.clone(),
            t.segments_done as usize,
            t.last_init,
        )
    };
    let request = Arc::new(request);
    let n = plan.segments.len();
    if next_write >= n {
        return Outcome::Done;
    }

    // Init segments are few and small; fetch them up front.
    let mut inits: Vec<Bytes> = Vec::with_capacity(plan.inits.len());
    for res in &plan.inits {
        match fetch::fetch_bytes(&ctx, &cancel, &request, res).await {
            Ok(b) => inits.push(b),
            Err(EngineError::Cancelled) => return Outcome::Cancelled,
            Err(e) => return Outcome::Failed(e),
        }
    }

    let mut file = match OpenOptions::new().append(true).open(&part_path) {
        Ok(f) => f,
        Err(e) => return Outcome::Failed(e.into()),
    };

    let max = ctx.max_connections.max(1) as usize;
    let window = (max * 2).clamp(2, 32);
    let mut next_fetch = next_write;
    let mut set: JoinSet<(usize, Result<Bytes>)> = JoinSet::new();
    let mut ready: BTreeMap<usize, Bytes> = BTreeMap::new();
    let mut buffered = 0usize;

    loop {
        while set.len() < max
            && next_fetch < n
            && next_fetch - next_write < window
            && (buffered < BUFFER_CAP || set.is_empty())
        {
            let idx = next_fetch;
            next_fetch += 1;
            let (c, tok, req, p, k) = (
                ctx.clone(),
                cancel.clone(),
                request.clone(),
                plan.clone(),
                keys.clone(),
            );
            set.spawn(async move { (idx, fetch_segment(&c, &tok, &req, &p, &k, idx).await) });
        }
        if next_write >= n {
            break;
        }

        if !ready.contains_key(&next_write) {
            let joined = tokio::select! {
                _ = cancel.cancelled() => {
                    set.abort_all();
                    return Outcome::Cancelled;
                }
                r = set.join_next() => r,
            };
            match joined {
                Some(Ok((idx, Ok(b)))) => {
                    buffered += b.len();
                    ready.insert(idx, b);
                    let mut m = ctx.lock_meta();
                    if let Some(st) = m.stream.as_mut() {
                        st.tracks[track_idx].ready.push(idx as u32);
                    }
                }
                Some(Ok((_, Err(e)))) => {
                    set.abort_all();
                    return if matches!(e, EngineError::Cancelled) {
                        Outcome::Cancelled
                    } else {
                        Outcome::Failed(e)
                    };
                }
                Some(Err(e)) => {
                    set.abort_all();
                    return Outcome::Failed(EngineError::Other(format!(
                        "segment task panicked: {e}"
                    )));
                }
                None => {
                    return Outcome::Failed(EngineError::Other("segment pipeline stalled".into()));
                }
            }
        }

        while let Some(b) = ready.remove(&next_write) {
            buffered -= b.len();
            let seg = &plan.segments[next_write];
            let mut wrote = 0u64;
            if seg.init_idx != last_init {
                if let Some(i) = seg.init_idx {
                    if let Err(e) = file.write_all(&inits[i as usize]) {
                        return Outcome::Failed(e.into());
                    }
                    wrote += inits[i as usize].len() as u64;
                }
                last_init = seg.init_idx;
            }
            if let Err(e) = file.write_all(&b) {
                return Outcome::Failed(e.into());
            }
            wrote += b.len() as u64;
            next_write += 1;
            let mut m = ctx.lock_meta();
            if let Some(st) = m.stream.as_mut() {
                let t = &mut st.tracks[track_idx];
                t.bytes_done += wrote;
                t.segments_done = next_write as u32;
                t.last_init = last_init;
                t.ready.retain(|&r| r as usize != next_write - 1);
            }
        }
    }
    if let Err(e) = file.flush() {
        return Outcome::Failed(e.into());
    }
    Outcome::Done
}

async fn fetch_segment(
    ctx: &RunCtx,
    cancel: &CancellationToken,
    req: &DownloadRequest,
    plan: &TrackPlan,
    keys: &KeyCache,
    idx: usize,
) -> Result<Bytes> {
    let seg = &plan.segments[idx];
    let raw = fetch::fetch_bytes(ctx, cancel, req, &seg.res).await?;
    match &seg.key {
        None => Ok(raw),
        Some(k) => {
            let key = keys.get(ctx, cancel, req, &k.url).await?;
            let plain = crypto::decrypt_aes128_cbc(&raw, &key, &k.iv)?;
            Ok(Bytes::from(plain))
        }
    }
}
