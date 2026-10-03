mod support;

use std::time::Duration;
use support::*;
use vfdm_engine::{DownloadRequest, DownloadStatus, Engine, Kind, Settings};

fn finished(s: &DownloadStatus) -> bool {
    matches!(
        s,
        DownloadStatus::Completed | DownloadStatus::Failed(_) | DownloadStatus::Cancelled
    )
}

#[tokio::test]
async fn two_tracks_without_ffmpeg_saves_separate_files() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let engine = Engine::new(Settings::new(out.clone()), &tmp.path().join("data")).unwrap();

    let id = engine
        .add(DownloadRequest {
            url: srv.url("dash/manifest.mpd"),
            ..Default::default()
        })
        .unwrap();
    let p = wait_status(&engine, id, finished, Duration::from_secs(30)).await;
    assert_eq!(p.status, DownloadStatus::Completed, "{p:?}");
    assert_eq!(p.kind, Kind::Dash);
    assert_eq!(p.segment_count, Some((DASH_V_SEGS + DASH_A_SEGS) as u32));
    assert_eq!(p.segments_done, p.segment_count);

    // Without ffmpeg: <stem>.video.mp4 + <stem>.audio.m4a, final_path = video.
    assert!(p.filename.ends_with(".video.mp4"), "{}", p.filename);
    let audio = out.join(p.filename.replace(".video.mp4", ".audio.m4a"));
    assert!(audio.exists(), "audio file missing: {}", audio.display());
    assert_eq!(sha256_file(&p.final_path), sha256(&srv.dash_video_plain()));
    assert_eq!(sha256_file(&audio), sha256(&srv.dash_audio_plain()));
    assert!(p.note.as_deref().unwrap_or("").contains("ffmpeg"));
    assert!(!out.join("dash.mp4.video.part").exists());
    assert!(!out.join("dash.mp4.audio.part").exists());
    engine.shutdown().await;
}

#[tokio::test]
async fn resumes_inside_second_track() {
    let srv = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let settings = Settings::new(tmp.path().join("out"));
    let engine = Engine::new(settings.clone(), &data).unwrap();
    let id = engine
        .add(DownloadRequest {
            url: srv.url("dash/manifest.mpd"),
            kind: Kind::Dash,
            ..Default::default()
        })
        .unwrap();
    // The fixture is small and fast; pause as soon as the video track has data.
    wait_downloaded(&engine, id, 1, Duration::from_secs(30)).await;
    let _ = engine.pause(id);
    let p = wait_status(
        &engine,
        id,
        |s| matches!(s, DownloadStatus::Paused | DownloadStatus::Completed),
        Duration::from_secs(10),
    )
    .await;
    engine.shutdown().await;
    drop(engine);

    let engine = Engine::new(settings, &data).unwrap();
    engine.load_all().unwrap();
    if p.status == DownloadStatus::Paused {
        engine.resume(id).unwrap();
    }
    let p = wait_status(&engine, id, finished, Duration::from_secs(60)).await;
    assert_eq!(p.status, DownloadStatus::Completed, "{p:?}");
    assert_eq!(sha256_file(&p.final_path), sha256(&srv.dash_video_plain()));
    engine.shutdown().await;
}
