mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;
use support::*;
use vfdm_engine::{DownloadRequest, DownloadStatus, Engine, Kind, Settings};

fn finished(s: &DownloadStatus) -> bool {
    matches!(
        s,
        DownloadStatus::Completed | DownloadStatus::Failed(_) | DownloadStatus::Cancelled
    )
}

async fn wait_segments_done(engine: &Engine, id: u64, at_least: u32, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let p = engine.get(id).unwrap();
        if p.segments_done.unwrap_or(0) >= at_least {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timeout waiting for segments; status {:?} done {:?}",
            p.status,
            p.segments_done
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn aes_ts_byte_exact_auto_detected() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let engine = Engine::new(Settings::new(out.clone()), &tmp.path().join("data")).unwrap();

    // kind defaults to Auto: the .m3u8 extension must route to the stream runner.
    let id = engine
        .add(DownloadRequest {
            url: srv.url("hls/media.m3u8"),
            ..Default::default()
        })
        .unwrap();
    let p = wait_status(&engine, id, finished, Duration::from_secs(30)).await;
    assert_eq!(p.status, DownloadStatus::Completed, "{p:?}");
    assert_eq!(p.kind, Kind::Hls);
    assert_eq!(p.segment_count, Some(HLS_SEGS as u32));
    assert_eq!(p.segments_done, Some(HLS_SEGS as u32));
    assert_eq!(
        p.filename, "hls.ts",
        "generic 'media' name falls back to parent segment"
    );
    assert_eq!(sha256_file(&p.final_path), sha256(&srv.hls_plain()));
    assert_eq!(p.total, Some((HLS_SEG * HLS_SEGS) as u64));
    assert!(!p.total_is_estimate);
    assert_eq!(
        srv.key_requests.load(Ordering::SeqCst),
        1,
        "key fetched once"
    );
    assert!(
        p.note.as_deref().unwrap_or("").contains("ffmpeg"),
        "no ffmpeg in tests → note"
    );
    assert!(!out.join("hls.ts.part").exists());
    engine.shutdown().await;
}

#[tokio::test]
async fn master_picks_highest_bandwidth() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::new(
        Settings::new(tmp.path().join("out")),
        &tmp.path().join("data"),
    )
    .unwrap();
    let id = engine
        .add(DownloadRequest {
            url: srv.url("hls/master.m3u8"),
            kind: Kind::Hls,
            ..Default::default()
        })
        .unwrap();
    let p = wait_status(&engine, id, finished, Duration::from_secs(30)).await;
    assert_eq!(p.status, DownloadStatus::Completed, "{p:?}");
    assert_eq!(
        sha256_file(&p.final_path),
        sha256(&srv.hls_plain()),
        "high variant content"
    );
    assert_ne!(sha256_file(&p.final_path), sha256(&srv.hls_low_plain()));
    engine.shutdown().await;
}

#[tokio::test]
async fn fmp4_byterange_with_init_map() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::new(
        Settings::new(tmp.path().join("out")),
        &tmp.path().join("data"),
    )
    .unwrap();
    let id = engine
        .add(DownloadRequest {
            url: srv.url("hls-fmp4/media.m3u8"),
            ..Default::default()
        })
        .unwrap();
    let p = wait_status(&engine, id, finished, Duration::from_secs(30)).await;
    assert_eq!(p.status, DownloadStatus::Completed, "{p:?}");
    assert!(p.filename.ends_with(".mp4"), "{}", p.filename);
    assert_eq!(sha256_file(&p.final_path), sha256(&srv.fmp4_plain()));
    assert!(srv.range_requests.load(Ordering::SeqCst) > FMP4_SEGS as u64);
    engine.shutdown().await;
}

#[tokio::test]
async fn pause_reload_resume_mid_stream() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let mut settings = Settings::new(tmp.path().join("out"));
    settings.max_connections = 2;

    let engine = Engine::new(settings.clone(), &data).unwrap();
    let id = engine
        .add(DownloadRequest {
            url: format!("{}?delay=60", srv.url("hls/media.m3u8")),
            ..Default::default()
        })
        .unwrap();
    wait_segments_done(&engine, id, 2, Duration::from_secs(30)).await;
    engine.pause(id).unwrap();
    let p = wait_status(
        &engine,
        id,
        |s| *s == DownloadStatus::Paused,
        Duration::from_secs(10),
    )
    .await;
    let done_before = p.segments_done.unwrap();
    assert!(done_before >= 2 && done_before < HLS_SEGS as u32, "{p:?}");
    engine.shutdown().await;
    drop(engine);

    let engine = Engine::new(settings, &data).unwrap();
    engine.load_all().unwrap();
    let p = engine.get(id).unwrap();
    assert_eq!(p.status, DownloadStatus::Paused);
    assert_eq!(p.segments_done, Some(done_before));
    let key_before = srv.key_requests.load(Ordering::SeqCst);

    engine.resume(id).unwrap();
    let p = wait_status(&engine, id, finished, Duration::from_secs(60)).await;
    assert_eq!(p.status, DownloadStatus::Completed, "{p:?}");
    assert_eq!(sha256_file(&p.final_path), sha256(&srv.hls_plain()));
    assert_eq!(
        srv.key_requests.load(Ordering::SeqCst),
        key_before + 1,
        "key re-fetched once per run"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn live_playlist_fails_clearly() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::new(
        Settings::new(tmp.path().join("out")),
        &tmp.path().join("data"),
    )
    .unwrap();
    let id = engine
        .add(DownloadRequest {
            url: srv.url("hls/live.m3u8"),
            ..Default::default()
        })
        .unwrap();
    let p = wait_status(&engine, id, finished, Duration::from_secs(30)).await;
    assert!(
        matches!(p.status, DownloadStatus::Failed(ref m) if m.contains("live")),
        "{p:?}"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn manifest_changed_on_resume_fails() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut settings = Settings::new(tmp.path().join("out"));
    settings.max_connections = 2;
    let engine = Engine::new(settings, &tmp.path().join("data")).unwrap();
    let id = engine
        .add(DownloadRequest {
            url: format!("{}?delay=60", srv.url("hls/media.m3u8")),
            ..Default::default()
        })
        .unwrap();
    wait_segments_done(&engine, id, 1, Duration::from_secs(30)).await;
    engine.pause(id).unwrap();
    wait_status(
        &engine,
        id,
        |s| *s == DownloadStatus::Paused,
        Duration::from_secs(10),
    )
    .await;

    reqwest::get(format!("{}/hls/flip", srv.base))
        .await
        .unwrap();
    engine.resume(id).unwrap();
    let p = wait_status(&engine, id, finished, Duration::from_secs(30)).await;
    assert!(
        matches!(p.status, DownloadStatus::Failed(ref m) if m.contains("changed")),
        "{p:?}"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn sidecar_v1_loads_as_file_kind() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(data.join("sidecars")).unwrap();
    let v1 = serde_json::json!({
        "version": 1, "id": 7,
        "request": { "url": "https://example.com/a.zip" },
        "final_url": "https://example.com/a.zip", "filename": "a.zip",
        "part_path": tmp.path().join("a.zip.part"), "final_path": tmp.path().join("a.zip"),
        "total": 100, "etag": null, "last_modified": null, "resumable": true,
        "segments": [{ "id": 0, "start": 0, "end": 99, "downloaded": 40 }],
        "status": { "state": "downloading" }, "created_at": 1, "completed_at": null
    });
    std::fs::write(
        data.join("sidecars/7.json"),
        serde_json::to_vec(&v1).unwrap(),
    )
    .unwrap();

    let engine = Engine::new(Settings::new(tmp.path().join("out")), &data).unwrap();
    assert_eq!(engine.load_all().unwrap(), 1);
    let p = engine.get(7).unwrap();
    assert_eq!(p.kind, Kind::File);
    assert_eq!(p.status, DownloadStatus::Paused);
    assert_eq!(p.downloaded, 40);
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(data.join("sidecars/7.json")).unwrap()).unwrap();
    assert_eq!(saved["version"], 2);
    assert_eq!(saved["kind"], "file");
    engine.shutdown().await;
}
