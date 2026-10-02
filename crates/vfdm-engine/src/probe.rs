use crate::error::{EngineError, Result};
use crate::filename;
use crate::http::apply_headers;
use crate::types::DownloadRequest;
use reqwest::header::{
    HeaderMap, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG,
    LAST_MODIFIED, RANGE,
};
use reqwest::{Client, StatusCode};
use url::Url;

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub final_url: Url,
    pub total: Option<u64>,
    pub accepts_ranges: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_type: Option<String>,
    pub filename: String,
}

/// `GET` with `Range: bytes=0-0` rather than `HEAD`: many CDNs mishandle HEAD,
/// and a 1-byte range answers size, range support and headers in one round trip.
pub async fn probe(client: &Client, req: &DownloadRequest) -> Result<ProbeResult> {
    let url = Url::parse(&req.url).map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
    let resp = apply_headers(client.get(url), req)
        .header(RANGE, "bytes=0-0")
        .send()
        .await?;

    let status = resp.status();
    let headers = resp.headers().clone();
    let final_url = resp.url().clone();
    drop(resp);

    let (total, accepts_ranges) = match status {
        StatusCode::PARTIAL_CONTENT => (parse_content_range_total(&headers), true),
        StatusCode::OK => (header_u64(&headers, CONTENT_LENGTH), false),
        StatusCode::RANGE_NOT_SATISFIABLE => (Some(0), true),
        s if s.is_client_error() || s.is_server_error() => {
            return Err(EngineError::Status(s.as_u16()))
        }
        _ => (header_u64(&headers, CONTENT_LENGTH), false),
    };

    let content_type = header_str(&headers, CONTENT_TYPE);
    let filename = filename::resolve(
        req.filename.as_deref(),
        header_str(&headers, CONTENT_DISPOSITION).as_deref(),
        &final_url,
        content_type.as_deref(),
    );

    Ok(ProbeResult {
        final_url,
        total,
        accepts_ranges: accepts_ranges && total.is_some(),
        etag: header_str(&headers, ETAG),
        last_modified: header_str(&headers, LAST_MODIFIED),
        content_type,
        filename,
    })
}

/// `Content-Range: bytes 0-0/12345` → 12345. `*` total → None.
pub fn parse_content_range_total(headers: &HeaderMap) -> Option<u64> {
    let v = headers.get(CONTENT_RANGE)?.to_str().ok()?;
    let rest = v.trim().strip_prefix("bytes")?.trim();
    let (_, total) = rest.rsplit_once('/')?;
    total.trim().parse().ok()
}

/// `Content-Range: bytes 100-199/12345` → Some((100, 199)).
pub fn parse_content_range_start_end(headers: &HeaderMap) -> Option<(u64, u64)> {
    let v = headers.get(CONTENT_RANGE)?.to_str().ok()?;
    let rest = v.trim().strip_prefix("bytes")?.trim();
    let (range, _) = rest.rsplit_once('/')?;
    let (s, e) = range.split_once('-')?;
    Some((s.trim().parse().ok()?, e.trim().parse().ok()?))
}

fn header_u64(h: &HeaderMap, k: reqwest::header::HeaderName) -> Option<u64> {
    h.get(k)?.to_str().ok()?.trim().parse().ok()
}

fn header_str(h: &HeaderMap, k: reqwest::header::HeaderName) -> Option<String> {
    h.get(k)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}
