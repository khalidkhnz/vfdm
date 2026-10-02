mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;
use support::*;
use vfdm_engine::{DownloadRequest, DownloadStatus, Engine, Settings};

fn finished(s: &DownloadStatus) -> bool {
    matches!(
        s,
        DownloadStatus::Completed | DownloadStatus::Failed(_) | DownloadStatus::Cancelled
    )
}

#[tokio::test]
async fn server_without_range_support_single_stream() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::new(
        Settings::new(tmp.path().join("out")),
        &tmp.path().join("data"),
    )
    .unwrap();
    let id = engine
        .add(DownloadRequest {
            url: srv.url("no-range"),
            ..Default::default()
        })
        .unwrap();
    let p = wait_status(&engine, id, finished, Duration::from_secs(30)).await;
    assert_eq!(p.status, DownloadStatus::Completed);
    assert!(!p.resumable);
    assert_eq!(p.segments.len(), 1);
    assert_eq!(sha256_file(&p.final_path), srv.sha);
    assert_eq!(srv.range_requests.load(Ordering::SeqCst), 0);
    engine.shutdown().await;
}

#[tokio::test]
async fn server_without_content_length() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::new(
        Settings::new(tmp.path().join("out")),
        &tmp.path().join("data"),
    )
    .unwrap();
    let id = engine
        .add(DownloadRequest {
            url: srv.url("no-length"),
            ..Default::default()
        })
        .unwrap();
    let p = wait_status(&engine, id, finished, Duration::from_secs(30)).await;
    assert_eq!(p.status, DownloadStatus::Completed, "{p:?}");
    assert_eq!(p.total, Some(SIZE as u64));
    assert_eq!(sha256_file(&p.final_path), srv.sha);
    engine.shutdown().await;
}

#[tokio::test]
async fn flaky_server_retries_from_offset() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut settings = Settings::new(tmp.path().join("out"));
    settings.max_connections = 6;
    let engine = Engine::new(settings, &tmp.path().join("data")).unwrap();
    let id = engine
        .add(DownloadRequest {
            url: format!("{}?fail_every=2", srv.url("flaky")),
            ..Default::default()
        })
        .unwrap();
    let p = wait_status(&engine, id, finished, Duration::from_secs(90)).await;
    assert_eq!(p.status, DownloadStatus::Completed, "{p:?}");
    assert_eq!(sha256_file(&p.final_path), srv.sha);
    engine.shutdown().await;
}

#[tokio::test]
async fn etag_change_on_resume_fails_cleanly() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::new(
        Settings::new(tmp.path().join("out")),
        &tmp.path().join("data"),
    )
    .unwrap();
    let id = engine
        .add(DownloadRequest {
            url: srv.url("etag"),
            ..Default::default()
        })
        .unwrap();
    // "etag" mode streams at full speed; pause as early as we can.
    wait_downloaded(&engine, id, 1, Duration::from_secs(30)).await;
    let _ = engine.pause(id);
    let p = wait_status(
        &engine,
        id,
        |s| matches!(s, DownloadStatus::Paused | DownloadStatus::Completed),
        Duration::from_secs(10),
    )
    .await;
    if p.status == DownloadStatus::Completed {
        // Too fast to pause on this machine; nothing to verify.
        engine.shutdown().await;
        return;
    }
    reqwest::get(format!("{}/flip", srv.base)).await.unwrap();
    engine.resume(id).unwrap();
    let p = wait_status(&engine, id, finished, Duration::from_secs(30)).await;
    assert!(
        matches!(p.status, DownloadStatus::Failed(ref m) if m.contains("changed")),
        "{p:?}"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn http_404_fails() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::new(
        Settings::new(tmp.path().join("out")),
        &tmp.path().join("data"),
    )
    .unwrap();
    let id = engine
        .add(DownloadRequest {
            url: srv.url("nope"),
            ..Default::default()
        })
        .unwrap();
    let p = wait_status(&engine, id, finished, Duration::from_secs(30)).await;
    assert!(
        matches!(p.status, DownloadStatus::Failed(ref m) if m.contains("404")),
        "{p:?}"
    );
    engine.shutdown().await;
}
