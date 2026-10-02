mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;
use support::*;
use vfdm_engine::{DownloadRequest, DownloadStatus, Engine, Settings};

#[tokio::test]
async fn eight_segments_byte_exact() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let mut settings = Settings::new(out.clone());
    settings.max_connections = 8;
    let engine = Engine::new(settings, &tmp.path().join("data")).unwrap();

    let id = engine
        .add(DownloadRequest {
            url: srv.url("file"),
            ..Default::default()
        })
        .unwrap();
    let p = wait_status(
        &engine,
        id,
        |s| {
            !matches!(
                s,
                DownloadStatus::Queued | DownloadStatus::Probing | DownloadStatus::Downloading
            )
        },
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(p.status, DownloadStatus::Completed, "{p:?}");
    assert_eq!(p.filename, "blob.bin");
    assert_eq!(p.final_path, out.join("blob.bin"));
    assert_eq!(sha256_file(&p.final_path), srv.sha);
    assert!(!out.join("blob.bin.part").exists());
    assert!(
        srv.range_requests.load(Ordering::SeqCst) >= 8,
        "expected parallel range requests"
    );
    assert!(p.resumable);
    assert!(p.segments.len() >= 8);
    engine.shutdown().await;
}

#[tokio::test]
async fn work_stealing_splits_segments() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut settings = Settings::new(tmp.path().join("out"));
    settings.max_connections = 2;
    let engine = Engine::new(settings, &tmp.path().join("data")).unwrap();

    // Two 4 MiB segments; the second half is throttled, so the first worker
    // finishes early and must steal the back half of the slow segment.
    let id = engine
        .add(DownloadRequest {
            url: srv.url("uneven"),
            ..Default::default()
        })
        .unwrap();
    let p = wait_status(
        &engine,
        id,
        |s| matches!(s, DownloadStatus::Completed | DownloadStatus::Failed(_)),
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(p.status, DownloadStatus::Completed);
    assert_eq!(sha256_file(&p.final_path), srv.sha);
    assert!(
        p.segments.len() > 2,
        "expected steals, got {} segments",
        p.segments.len()
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn duplicate_filename_gets_suffix() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("blob.bin"), b"existing").unwrap();
    let engine = Engine::new(Settings::new(out.clone()), &tmp.path().join("data")).unwrap();
    let id = engine
        .add(DownloadRequest {
            url: srv.url("file"),
            ..Default::default()
        })
        .unwrap();
    let p = wait_status(
        &engine,
        id,
        |s| s.is_terminal() || matches!(s, DownloadStatus::Failed(_)),
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(p.status, DownloadStatus::Completed);
    assert_eq!(p.final_path, out.join("blob (1).bin"));
    assert_eq!(std::fs::read(out.join("blob.bin")).unwrap(), b"existing");
    engine.shutdown().await;
}
