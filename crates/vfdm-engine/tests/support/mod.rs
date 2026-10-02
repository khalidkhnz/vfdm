#![allow(dead_code)]

use aes::cipher::{Array, BlockCipherEncrypt, KeyInit};
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
    pub key_requests: AtomicU64,
    pub hls_flipped: AtomicBool,
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
            key_requests: AtomicU64::new(0),
            hls_flipped: AtomicBool::new(false),
        });
        let app = Router::new()
            .route("/flip", get(flip))
            .route("/hls/flip", get(hls_flip))
            .route("/hls/{*rest}", get(hls))
            .route("/hls-fmp4/{*rest}", get(hls_fmp4))
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

// ---------------------------------------------------------------------------
// HLS fixtures. Segment i of the keyed playlist is data[i*SEG..(i+1)*SEG]
// encrypted with AES-128-CBC, IV = sequence number. The fMP4 playlist slices
// one file by byte range with a 1000-byte init map.

pub const HLS_SEG: usize = 300 * 1024;
pub const HLS_SEGS: usize = 6;
pub const HLS_KEY: [u8; 16] = [7u8; 16];
pub const HLS_LOW_OFFSET: usize = 4 * 1024 * 1024;
pub const FMP4_INIT: usize = 1000;
pub const FMP4_SEG: usize = 200_000;
pub const FMP4_SEGS: usize = 4;

impl TestServer {
    pub fn hls_plain(&self) -> Bytes {
        self.data.slice(0..HLS_SEG * HLS_SEGS)
    }
    pub fn hls_low_plain(&self) -> Bytes {
        self.data
            .slice(HLS_LOW_OFFSET..HLS_LOW_OFFSET + HLS_SEG * 2)
    }
    pub fn fmp4_plain(&self) -> Bytes {
        self.data.slice(0..FMP4_INIT + FMP4_SEG * FMP4_SEGS)
    }
}

pub fn sha256(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

pub fn aes_cbc_encrypt(plain: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Vec<u8> {
    let cipher = aes::Aes128::new(&Array::from(*key));
    let pad = 16 - plain.len() % 16;
    let mut buf = plain.to_vec();
    buf.extend(std::iter::repeat_n(pad as u8, pad));
    let mut prev = *iv;
    for chunk in buf.chunks_exact_mut(16) {
        for i in 0..16 {
            chunk[i] ^= prev[i];
        }
        let mut block = Array::from(<[u8; 16]>::try_from(&*chunk).unwrap());
        cipher.encrypt_block(&mut block);
        chunk.copy_from_slice(&block.0);
        prev.copy_from_slice(chunk);
    }
    buf
}

fn seq_iv(seq: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&seq.to_be_bytes());
    iv
}

async fn hls_flip(State(s): State<Arc<TestServer>>) -> StatusCode {
    s.hls_flipped.store(true, Ordering::SeqCst);
    StatusCode::NO_CONTENT
}

fn m3u8_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        header::CONTENT_TYPE,
        "application/vnd.apple.mpegurl".parse().unwrap(),
    );
    h
}

async fn hls(
    State(s): State<Arc<TestServer>>,
    Path(rest): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    s.requests.fetch_add(1, Ordering::SeqCst);
    let delay: u64 = q.get("delay").and_then(|v| v.parse().ok()).unwrap_or(0);
    let qs = if delay > 0 {
        format!("?delay={delay}")
    } else {
        String::new()
    };
    match rest.as_str() {
        "master.m3u8" => {
            let body = format!(
                "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=640x360\nlow.m3u8{qs}\n#EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720\nmedia.m3u8{qs}\n"
            );
            (StatusCode::OK, m3u8_headers(), body).into_response()
        }
        "media.m3u8" | "live.m3u8" => {
            let n = if s.hls_flipped.load(Ordering::SeqCst) {
                HLS_SEGS - 1
            } else {
                HLS_SEGS
            };
            let mut body = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n");
            for i in 0..n {
                body.push_str(&format!("#EXTINF:10.0,\nseg/{i}.ts{qs}\n"));
            }
            if rest == "media.m3u8" {
                body.push_str("#EXT-X-ENDLIST\n");
            }
            (StatusCode::OK, m3u8_headers(), body).into_response()
        }
        "low.m3u8" => {
            let body = format!(
                "#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:10.0,\nlowseg/0.ts{qs}\n#EXTINF:10.0,\nlowseg/1.ts{qs}\n#EXT-X-ENDLIST\n"
            );
            (StatusCode::OK, m3u8_headers(), body).into_response()
        }
        "key.bin" => {
            s.key_requests.fetch_add(1, Ordering::SeqCst);
            (StatusCode::OK, Body::from(HLS_KEY.to_vec())).into_response()
        }
        p if p.starts_with("seg/") => {
            let i: usize = p
                .trim_start_matches("seg/")
                .trim_end_matches(".ts")
                .parse()
                .unwrap();
            let plain = s.data.slice(i * HLS_SEG..(i + 1) * HLS_SEG);
            let enc = Bytes::from(aes_cbc_encrypt(&plain, &HLS_KEY, &seq_iv(i as u64)));
            let mut h = HeaderMap::new();
            h.insert(header::CONTENT_TYPE, "video/mp2t".parse().unwrap());
            h.insert(
                header::CONTENT_LENGTH,
                enc.len().to_string().parse().unwrap(),
            );
            (
                StatusCode::OK,
                h,
                throttled(enc, Duration::from_millis(delay), None),
            )
                .into_response()
        }
        p if p.starts_with("lowseg/") => {
            let i: usize = p
                .trim_start_matches("lowseg/")
                .trim_end_matches(".ts")
                .parse()
                .unwrap();
            let plain = s
                .data
                .slice(HLS_LOW_OFFSET + i * HLS_SEG..HLS_LOW_OFFSET + (i + 1) * HLS_SEG);
            (StatusCode::OK, Body::from(plain)).into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn hls_fmp4(
    State(s): State<Arc<TestServer>>,
    Path(rest): Path<String>,
    req: Request<Body>,
) -> Response {
    s.requests.fetch_add(1, Ordering::SeqCst);
    match rest.as_str() {
        "media.m3u8" => {
            let mut body = format!(
                "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:4\n#EXT-X-MAP:URI=\"all.bin\",BYTERANGE=\"{FMP4_INIT}@0\"\n"
            );
            for i in 0..FMP4_SEGS {
                if i == 0 {
                    body.push_str(&format!(
                        "#EXTINF:4.0,\n#EXT-X-BYTERANGE:{FMP4_SEG}@{FMP4_INIT}\nall.bin\n"
                    ));
                } else {
                    body.push_str(&format!(
                        "#EXTINF:4.0,\n#EXT-X-BYTERANGE:{FMP4_SEG}\nall.bin\n"
                    ));
                }
            }
            body.push_str("#EXT-X-ENDLIST\n");
            (StatusCode::OK, m3u8_headers(), body).into_response()
        }
        "all.bin" => {
            let all = s.fmp4_plain();
            let len = all.len() as u64;
            match parse_range(req.headers(), len) {
                Some((start, end)) => {
                    s.range_requests.fetch_add(1, Ordering::SeqCst);
                    let slice = all.slice(start as usize..=end as usize);
                    let mut h = HeaderMap::new();
                    h.insert(
                        header::CONTENT_RANGE,
                        format!("bytes {start}-{end}/{len}").parse().unwrap(),
                    );
                    h.insert(
                        header::CONTENT_LENGTH,
                        slice.len().to_string().parse().unwrap(),
                    );
                    (StatusCode::PARTIAL_CONTENT, h, Body::from(slice)).into_response()
                }
                None => (StatusCode::OK, Body::from(all)).into_response(),
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
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
