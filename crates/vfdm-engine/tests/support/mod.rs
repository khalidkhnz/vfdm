#![allow(dead_code)]

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use rand::{rngs::StdRng, RngCore, SeedableRng};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const SIZE: usize = 8 * 1024 * 1024;

pub struct TestServer {
    pub data: Bytes,
    pub sha: [u8; 32],
    pub base: String,
    pub requests: AtomicU64,
    pub range_requests: AtomicU64,
    pub etag_flipped: AtomicBool,
}

impl TestServer {
    pub async fn start() -> Arc<Self> {
        let mut rng = StdRng::seed_from_u64(42);
        let mut buf = vec![0u8; SIZE];
        rng.fill_bytes(&mut buf);
        let sha: [u8; 32] = Sha256::digest(&buf).into();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = Arc::new(TestServer {
            data: Bytes::from(buf),
            sha,
            base: format!("http://127.0.0.1:{port}"),
            requests: AtomicU64::new(0),
            range_requests: AtomicU64::new(0),
            etag_flipped: AtomicBool::new(false),
        });
        let app = Router::new()
            .route("/flip", get(flip))
            .route("/{mode}", get(serve))
            .with_state(server.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        server
    }

    pub fn url(&self, mode: &str) -> String {
        format!("{}/{mode}", self.base)
    }

    pub fn etag(&self) -> &'static str {
        if self.etag_flipped.load(Ordering::SeqCst) {
            "\"v2\""
        } else {
            "\"v1\""
        }
    }
}

async fn flip(State(s): State<Arc<TestServer>>) -> StatusCode {
    s.etag_flipped.store(true, Ordering::SeqCst);
    StatusCode::NO_CONTENT
}

fn parse_range(h: &HeaderMap, len: u64) -> Option<(u64, u64)> {
    let v = h.get(header::RANGE)?.to_str().ok()?;
    let spec = v.strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    let start: u64 = a.parse().ok()?;
    let end: u64 = if b.is_empty() {
        len - 1
    } else {
        b.parse().ok()?
    };
    (start <= end && end < len).then_some((start, end))
}

async fn serve(
    State(s): State<Arc<TestServer>>,
    Path(mode): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    req: Request<Body>,
) -> Response {
    let n = s.requests.fetch_add(1, Ordering::SeqCst) + 1;
    let headers = req.headers();
    let len = s.data.len() as u64;
    let etag = s.etag();

    let mut common = HeaderMap::new();
    common.insert(
        header::CONTENT_TYPE,
        "application/octet-stream".parse().unwrap(),
    );
    common.insert(
        header::CONTENT_DISPOSITION,
        "attachment; filename=\"blob.bin\"".parse().unwrap(),
    );

    match mode.as_str() {
        "no-range" => {
            common.insert(header::CONTENT_LENGTH, len.to_string().parse().unwrap());
            (StatusCode::OK, common, Body::from(s.data.clone())).into_response()
        }
        "no-length" => {
            let data = s.data.clone();
            let stream =
                futures_util::stream::iter((0..data.len()).step_by(64 * 1024).map(move |i| {
                    Ok::<_, std::io::Error>(data.slice(i..(i + 64 * 1024).min(data.len())))
                }));
            (StatusCode::OK, common, Body::from_stream(stream)).into_response()
        }
        "file" | "slow" | "flaky" | "etag" | "uneven" => {
            common.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
            common.insert(header::ETAG, etag.parse().unwrap());

            if mode == "etag" {
                if let Some(ir) = headers.get(header::IF_RANGE).and_then(|v| v.to_str().ok()) {
                    if ir != etag {
                        common.insert(header::CONTENT_LENGTH, len.to_string().parse().unwrap());
                        return (StatusCode::OK, common, Body::from(s.data.clone()))
                            .into_response();
                    }
                }
            }

            let (start, end) = match parse_range(headers, len) {
                Some(r) => {
                    s.range_requests.fetch_add(1, Ordering::SeqCst);
                    r
                }
                None => {
                    if headers.contains_key(header::RANGE) {
                        common.insert(
                            header::CONTENT_RANGE,
                            format!("bytes */{len}").parse().unwrap(),
                        );
                        return (StatusCode::RANGE_NOT_SATISFIABLE, common).into_response();
                    }
                    (0, len - 1)
                }
            };
            let slice = s.data.slice(start as usize..=end as usize);
            let status = if headers.contains_key(header::RANGE) {
                StatusCode::PARTIAL_CONTENT
            } else {
                StatusCode::OK
            };
            common.insert(
                header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{len}").parse().unwrap(),
            );
            common.insert(
                header::CONTENT_LENGTH,
                slice.len().to_string().parse().unwrap(),
            );

            let fail_every: u64 = q
                .get("fail_every")
                .and_then(|v| v.parse().ok())
                .unwrap_or(3);
            let body = match mode.as_str() {
                "slow" => throttled(slice, Duration::from_millis(15), None),
                "uneven" if start >= len / 2 => throttled(slice, Duration::from_millis(15), None),
                "flaky" if n % fail_every == 0 => {
                    throttled(slice, Duration::ZERO, Some(100 * 1024))
                }
                _ => Body::from(slice),
            };
            (status, common, body).into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// 64 KiB chunks with a delay between them; optionally errors out after `cut` bytes.
fn throttled(data: Bytes, delay: Duration, cut: Option<usize>) -> Body {
    let stream = async_stream_chunks(data, delay, cut);
    Body::from_stream(stream)
}

fn async_stream_chunks(
    data: Bytes,
    delay: Duration,
    cut: Option<usize>,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> {
    futures_util::stream::unfold(0usize, move |pos| {
        let data = data.clone();
        async move {
            if pos >= data.len() {
                return None;
            }
            if let Some(c) = cut {
                if pos >= c {
                    return Some((Err(std::io::Error::other("simulated drop")), data.len()));
                }
            }
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let end = (pos + 64 * 1024).min(data.len());
            Some((Ok(data.slice(pos..end)), end))
        }
    })
}

pub fn sha256_file(p: &std::path::Path) -> [u8; 32] {
    Sha256::digest(std::fs::read(p).unwrap()).into()
}

pub async fn wait_status<F: Fn(&vfdm_engine::DownloadStatus) -> bool>(
    engine: &vfdm_engine::Engine,
    id: u64,
    pred: F,
    timeout: Duration,
) -> vfdm_engine::Progress {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let p = engine.get(id).expect("download exists");
        if pred(&p.status) {
            return p;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timeout waiting; last status {:?}",
            p.status
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub async fn wait_downloaded(
    engine: &vfdm_engine::Engine,
    id: u64,
    at_least: u64,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let p = engine.get(id).unwrap();
        if p.downloaded >= at_least {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timeout waiting for bytes; status {:?} downloaded {}",
            p.status,
            p.downloaded
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
