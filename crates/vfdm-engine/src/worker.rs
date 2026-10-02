use crate::download::RunCtx;
use crate::error::EngineError;
use crate::http::apply_headers;
use crate::probe::parse_content_range_start_end;
use crate::writer::{open_for_write, preallocate, write_at};
use futures_util::StreamExt;
use rand::Rng;
use reqwest::header::{CONTENT_LENGTH, IF_RANGE, RANGE};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use url::Url;

const MAX_ATTEMPTS: u32 = 8;
const STALL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub enum Outcome {
    Done,
    Cancelled,
    /// Server answered 200 to a ranged request with no validator mismatch.
    RangeIgnored,
    /// Server answered 200 to a ranged request after `If-Range`: file changed.
    RemoteChanged,
    Failed(EngineError),
}

fn backoff(attempt: u32) -> Duration {
    let base = Duration::from_secs(1) * 2u32.pow(attempt.min(5));
    let jitter = Duration::from_millis(rand::rng().random_range(0..500));
    (base + jitter).min(Duration::from_secs(30))
}

/// Fetches one segment with `Range`, writing at absolute offsets. Re-reads the
/// segment's `end` before every chunk so a steal mid-stream is honoured.
pub async fn run_segment(ctx: Arc<RunCtx>, cancel: CancellationToken, seg_id: u32) -> Outcome {
    let (request, final_url, etag, last_modified, part_path) = {
        let m = ctx.lock_meta();
        (
            m.request.clone(),
            m.final_url.clone(),
            m.etag.clone(),
            m.last_modified.clone(),
            m.part_path.clone(),
        )
    };
    let url = match Url::parse(&final_url) {
        Ok(u) => u,
        Err(e) => return Outcome::Failed(EngineError::InvalidUrl(e.to_string())),
    };
    let validator = etag.as_ref().or(last_modified.as_ref()).cloned();

    let mut attempt = 0u32;
    let mut last_err: EngineError;

    loop {
        if cancel.is_cancelled() {
            return Outcome::Cancelled;
        }
        let (from, end) = {
            let m = ctx.lock_meta();
            match m.segments.iter().find(|s| s.id == seg_id) {
                Some(s) if s.is_done() => return Outcome::Done,
                Some(s) => (s.next(), s.end),
                None => {
                    return Outcome::Failed(EngineError::Other(format!(
                        "segment {seg_id} vanished"
                    )))
                }
            }
        };
        if attempt > 0 {
            tokio::select! {
                _ = cancel.cancelled() => return Outcome::Cancelled,
                _ = tokio::time::sleep(backoff(attempt)) => {}
            }
        }

        let _permit = tokio::select! {
            _ = cancel.cancelled() => return Outcome::Cancelled,
            p = ctx.host_sem.acquire() => p,
        };

        let mut rb = apply_headers(ctx.client.get(url.clone()), &request)
            .header(RANGE, format!("bytes={from}-{end}"));
        if let Some(v) = &validator {
            rb = rb.header(IF_RANGE, v);
        }
        let resp = tokio::select! {
            _ = cancel.cancelled() => return Outcome::Cancelled,
            r = rb.send() => r,
        };
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                last_err = e.into();
                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    return Outcome::Failed(last_err);
                }
                continue;
            }
        };

        match resp.status().as_u16() {
            206 => {
                if let Some((s, _)) = parse_content_range_start_end(resp.headers()) {
                    if s != from {
                        return Outcome::Failed(EngineError::RangeMismatch);
                    }
                }
            }
            200 => {
                return if from > 0 && validator.is_some() {
                    Outcome::RemoteChanged
                } else {
                    Outcome::RangeIgnored
                };
            }
            416 => return Outcome::Failed(EngineError::RangeMismatch),
            s @ (408 | 429 | 500..=599) => {
                last_err = EngineError::Status(s);
                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    return Outcome::Failed(last_err);
                }
                continue;
            }
            s => return Outcome::Failed(EngineError::Status(s)),
        }

        let file = match open_for_write(&part_path) {
            Ok(f) => f,
            Err(e) => return Outcome::Failed(e.into()),
        };
        let mut stream = resp.bytes_stream();
        let mut pos = from;
        let mut stream_err: Option<EngineError> = None;

        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => return Outcome::Cancelled,
                r = tokio::time::timeout(STALL_TIMEOUT, stream.next()) => r,
            };
            match next {
                Err(_) => {
                    stream_err = Some(EngineError::Other("connection stalled".into()));
                    break;
                }
                Ok(None) => break,
                Ok(Some(Err(e))) => {
                    stream_err = Some(e.into());
                    break;
                }
                Ok(Some(Ok(chunk))) => {
                    let cur_end = {
                        let m = ctx.lock_meta();
                        m.segments
                            .iter()
                            .find(|s| s.id == seg_id)
                            .map(|s| s.end)
                            .unwrap_or(end)
                    };
                    if pos > cur_end {
                        break;
                    }
                    let allowed = ((cur_end + 1 - pos) as usize).min(chunk.len());
                    if let Err(e) = write_at(&file, pos, &chunk[..allowed]) {
                        return Outcome::Failed(e.into());
                    }
                    pos += allowed as u64;
                    {
                        let mut m = ctx.lock_meta();
                        if let Some(s) = m.segments.iter_mut().find(|s| s.id == seg_id) {
                            s.downloaded = (pos - s.start).min(s.len());
                        }
                    }
                    if pos > cur_end {
                        break;
                    }
                }
            }
        }
        drop(stream);

        let done = ctx
            .lock_meta()
            .segments
            .iter()
            .find(|s| s.id == seg_id)
            .map(|s| s.is_done())
            .unwrap_or(true);
        if done {
            return Outcome::Done;
        }
        // Progress resets the retry budget; a short body with no progress burns one.
        if pos > from {
            attempt = 0;
        } else {
            attempt += 1;
        }
        last_err = stream_err.unwrap_or_else(|| EngineError::Other("short body".into()));
        if attempt >= MAX_ATTEMPTS {
            return Outcome::Failed(last_err);
        }
    }
}

/// Non-resumable path: one plain GET, written sequentially. Any failure
/// restarts from zero because the server gave us no way to resume.
pub async fn run_single(ctx: Arc<RunCtx>, cancel: CancellationToken) -> Outcome {
    let (request, final_url, part_path, known_total) = {
        let m = ctx.lock_meta();
        (
            m.request.clone(),
            m.final_url.clone(),
            m.part_path.clone(),
            m.total,
        )
    };
    let url = match Url::parse(&final_url) {
        Ok(u) => u,
        Err(e) => return Outcome::Failed(EngineError::InvalidUrl(e.to_string())),
    };

    let mut attempt = 0u32;
    loop {
        if cancel.is_cancelled() {
            return Outcome::Cancelled;
        }
        if attempt > 0 {
            tokio::select! {
                _ = cancel.cancelled() => return Outcome::Cancelled,
                _ = tokio::time::sleep(backoff(attempt)) => {}
            }
        }
        let _permit = tokio::select! {
            _ = cancel.cancelled() => return Outcome::Cancelled,
            p = ctx.host_sem.acquire() => p,
        };
        let resp = tokio::select! {
            _ = cancel.cancelled() => return Outcome::Cancelled,
            r = apply_headers(ctx.client.get(url.clone()), &request).send() => r,
        };
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    return Outcome::Failed(e.into());
                }
                continue;
            }
        };
        match resp.status().as_u16() {
            200 | 206 => {}
            s @ (408 | 429 | 500..=599) => {
                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    return Outcome::Failed(EngineError::Status(s));
                }
                continue;
            }
            s => return Outcome::Failed(EngineError::Status(s)),
        }

        let total = known_total.or_else(|| {
            resp.headers()
                .get(CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
        });
        let file = match preallocate(&part_path, total.unwrap_or(0)) {
            Ok(f) => f,
            Err(e) => return Outcome::Failed(e.into()),
        };
        {
            let mut m = ctx.lock_meta();
            m.total = total;
            if let Some(s) = m.segments.first_mut() {
                s.start = 0;
                s.downloaded = 0;
                s.end = total.map(|t| t.saturating_sub(1)).unwrap_or(u64::MAX);
            }
        }

        let mut stream = resp.bytes_stream();
        let mut pos = 0u64;
        let mut failed = false;
        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => return Outcome::Cancelled,
                r = tokio::time::timeout(STALL_TIMEOUT, stream.next()) => r,
            };
            match next {
                Err(_) | Ok(Some(Err(_))) => {
                    failed = true;
                    break;
                }
                Ok(None) => break,
                Ok(Some(Ok(chunk))) => {
                    if let Err(e) = write_at(&file, pos, &chunk) {
                        return Outcome::Failed(e.into());
                    }
                    pos += chunk.len() as u64;
                    if let Some(s) = ctx.lock_meta().segments.first_mut() {
                        s.downloaded = pos;
                    }
                }
            }
        }
        drop(stream);

        match total {
            Some(t) if !failed && pos == t => return Outcome::Done,
            Some(_) => {
                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    return Outcome::Failed(EngineError::Other("short body".into()));
                }
            }
            None if !failed => {
                let mut m = ctx.lock_meta();
                m.total = Some(pos);
                if let Some(s) = m.segments.first_mut() {
                    s.end = pos.saturating_sub(1);
                    s.downloaded = pos;
                }
                return Outcome::Done;
            }
            None => {
                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    return Outcome::Failed(EngineError::Other("connection failed".into()));
                }
            }
        }
    }
}
