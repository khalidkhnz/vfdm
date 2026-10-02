mod support;

use std::time::Duration;
use support::*;
use vfdm_engine::{DownloadRequest, DownloadStatus, Engine, Settings};

#[tokio::test]
async fn pause_drop_engine_reload_resume() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let mut settings = Settings::new(tmp.path().join("out"));
    settings.max_connections = 4;

    let engine = Engine::new(settings.clone(), &data).unwrap();
    let id = engine
        .add(DownloadRequest {
            url: srv.url("slow"),
            ..Default::default()
        })
        .unwrap();
    wait_downloaded(&engine, id, (SIZE / 4) as u64, Duration::from_secs(30)).await;
    engine.pause(id).unwrap();
    let p = wait_status(
        &engine,
        id,
        |s| *s == DownloadStatus::Paused,
        Duration::from_secs(10),
    )
    .await;
    let before = p.downloaded;
    assert!(before >= (SIZE / 4) as u64 && before < SIZE as u64);
    engine.shutdown().await;
    drop(engine);

    let engine = Engine::new(settings, &data).unwrap();
    assert_eq!(engine.load_all().unwrap(), 1);
    let p = engine.get(id).unwrap();
    assert_eq!(p.status, DownloadStatus::Paused);
    assert_eq!(p.downloaded, before, "sidecar must carry progress");

    engine.resume(id).unwrap();
    let p = wait_status(
        &engine,
        id,
        |s| matches!(s, DownloadStatus::Completed | DownloadStatus::Failed(_)),
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(p.status, DownloadStatus::Completed);
    assert_eq!(sha256_file(&p.final_path), srv.sha);
    engine.shutdown().await;
}

#[tokio::test]
async fn crash_mid_download_reloads_as_paused() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let settings = Settings::new(tmp.path().join("out"));

    let engine = Engine::new(settings.clone(), &data).unwrap();
    let id = engine
        .add(DownloadRequest {
            url: srv.url("slow"),
            ..Default::default()
        })
        .unwrap();
    wait_downloaded(&engine, id, (SIZE / 8) as u64, Duration::from_secs(30)).await;
    engine.shutdown().await;
    drop(engine);

    // A crash leaves the sidecar saying "downloading" with partial progress. Forge that state.
    let sidecar = data.join("sidecars").join(format!("{id}.json"));
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
    assert_eq!(json["status"]["state"], "paused");
    json["status"] = serde_json::json!({ "state": "downloading" });
    let before = json["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["downloaded"].as_u64().unwrap())
        .sum::<u64>();
    assert!(before > 0);
    std::fs::write(&sidecar, serde_json::to_vec(&json).unwrap()).unwrap();

    let engine = Engine::new(settings, &data).unwrap();
    engine.load_all().unwrap();
    let p = engine.get(id).unwrap();
    assert_eq!(p.status, DownloadStatus::Paused);
    assert_eq!(p.downloaded, before);
    engine.resume(id).unwrap();
    let p = wait_status(
        &engine,
        id,
        |s| matches!(s, DownloadStatus::Completed | DownloadStatus::Failed(_)),
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(p.status, DownloadStatus::Completed);
    assert_eq!(sha256_file(&p.final_path), srv.sha);
    engine.shutdown().await;
}

#[tokio::test]
async fn cancel_with_delete_removes_part() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let engine = Engine::new(Settings::new(out.clone()), &tmp.path().join("data")).unwrap();
    let id = engine
        .add(DownloadRequest {
            url: srv.url("slow"),
            ..Default::default()
        })
        .unwrap();
    wait_downloaded(&engine, id, 1, Duration::from_secs(30)).await;
    assert!(out.join("blob.bin.part").exists());
    engine.cancel(id, true).unwrap();
    wait_status(
        &engine,
        id,
        |s| *s == DownloadStatus::Cancelled,
        Duration::from_secs(10),
    )
    .await;
    assert!(!out.join("blob.bin.part").exists());
    engine.shutdown().await;
}
