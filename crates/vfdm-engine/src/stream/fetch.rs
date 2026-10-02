use super::Resource;
use crate::download::RunCtx;
use crate::error::{EngineError, Result};
use crate::http::apply_headers;
use crate::types::DownloadRequest;
use crate::worker::{backoff, MAX_ATTEMPTS, STALL_TIMEOUT};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use reqwest::header::{CONTENT_LENGTH, RANGE};
use tokio_util::sync::CancellationToken;
use url::Url;

/// Whole-resource GET with the download's headers, per-host permit, retries
/// with backoff, stall timeout, and a Content-Length check.
pub async fn fetch_bytes(
    ctx: &RunCtx,
    cancel: &CancellationToken,
    req: &DownloadRequest,
    res: &Resource,
) -> Result<Bytes> {
    fetch_inner(ctx, cancel, req, res).await.map(|(b, _)| b)
}

/// Same, also returning the post-redirect URL (base for relative manifest URIs).
pub async fn fetch_bytes_url(
    ctx: &RunCtx,
    cancel: &CancellationToken,
    req: &DownloadRequest,
    res: &Resource,
) -> Result<(Bytes, Url)> {
    fetch_inner(ctx, cancel, req, res).await
}

async fn fetch_inner(
    ctx: &RunCtx,
    cancel: &CancellationToken,
    req: &DownloadRequest,
    res: &Resource,
) -> Result<(Bytes, Url)> {
    let mut attempt = 0u32;
    let mut last_err: EngineError;
    loop {
        if cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        if attempt > 0 {
            tokio::select! {
                _ = cancel.cancelled() => return Err(EngineError::Cancelled),
                _ = tokio::time::sleep(backoff(attempt)) => {}
            }
        }
        let _permit = tokio::select! {
            _ = cancel.cancelled() => return Err(EngineError::Cancelled),
            p = ctx.host_sem.acquire() => p,
        };
        let mut rb = apply_headers(ctx.client.get(res.url.clone()), req);
        if let Some((s, e)) = res.range {
            rb = rb.header(RANGE, format!("bytes={s}-{e}"));
        }
        let resp = tokio::select! {
            _ = cancel.cancelled() => return Err(EngineError::Cancelled),
            r = rb.send() => r,
        };
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                last_err = e.into();
                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    return Err(last_err);
                }
                continue;
            }
        };
        let status = resp.status().as_u16();
        match status {
            200 | 206 => {}
            s @ (408 | 429 | 500..=599) => {
                last_err = EngineError::Status(s);
                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    return Err(last_err);
                }
                continue;
            }
            s => return Err(EngineError::Status(s)),
        }
        if res.range.is_some() && status == 200 {
            return Err(EngineError::RangeIgnored);
        }
        let final_url = resp.url().clone();
        let expected: Option<u64> = resp
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok());

        let mut buf = BytesMut::with_capacity(expected.unwrap_or(64 * 1024).min(64 << 20) as usize);
        let mut stream = resp.bytes_stream();
        let mut failed: Option<EngineError> = None;
        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => return Err(EngineError::Cancelled),
                r = tokio::time::timeout(STALL_TIMEOUT, stream.next()) => r,
            };
            match next {
                Err(_) => {
                    failed = Some(EngineError::Other("connection stalled".into()));
                    break;
                }
                Ok(None) => break,
                Ok(Some(Err(e))) => {
                    failed = Some(e.into());
                    break;
                }
                Ok(Some(Ok(chunk))) => buf.extend_from_slice(&chunk),
            }
        }
        if failed.is_none() {
            if let Some(exp) = expected {
                if exp != buf.len() as u64 {
                    failed = Some(EngineError::Other(format!(
                        "short body: {} of {} bytes",
                        buf.len(),
                        exp
                    )));
                }
            }
        }
        match failed {
            None => return Ok((buf.freeze(), final_url)),
            Some(e) => {
                last_err = e;
                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    return Err(last_err);
                }
            }
        }
    }
}
