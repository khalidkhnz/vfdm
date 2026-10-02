use crate::error::{EngineError, Result};
use std::ffi::OsStr;
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

/// Runs ffmpeg to completion; kills it if the download is cancelled.
pub async fn run_ffmpeg<I, S>(ffmpeg: &Path, args: I, cancel: &CancellationToken) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| EngineError::Other(format!("cannot start ffmpeg: {e}")))?;
    let mut stderr = child.stderr.take();
    let read_err = async {
        let mut buf = String::new();
        if let Some(s) = stderr.as_mut() {
            let _ = s.read_to_string(&mut buf).await;
        }
        buf
    };
    tokio::select! {
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            Err(EngineError::Cancelled)
        }
        (status, err) = async { tokio::join!(child.wait(), read_err) } => {
            let status = status.map_err(|e| EngineError::Other(format!("ffmpeg wait failed: {e}")))?;
            if status.success() {
                Ok(())
            } else {
                let tail: String = err.lines().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ");
                Err(EngineError::Other(format!("ffmpeg exited with {status}: {tail}")))
            }
        }
    }
}

pub async fn remux_ts_to_mp4(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    cancel: &CancellationToken,
) -> Result<()> {
    run_ffmpeg(
        ffmpeg,
        [
            OsStr::new("-i"),
            input.as_os_str(),
            OsStr::new("-c"),
            OsStr::new("copy"),
            OsStr::new("-bsf:a"),
            OsStr::new("aac_adtstoasc"),
            OsStr::new("-movflags"),
            OsStr::new("+faststart"),
            OsStr::new("-f"),
            OsStr::new("mp4"),
            output.as_os_str(),
        ],
        cancel,
    )
    .await
}

pub async fn mux_av(
    ffmpeg: &Path,
    video: &Path,
    audio: &Path,
    output: &Path,
    cancel: &CancellationToken,
) -> Result<()> {
    let fmt = if output.extension().and_then(|e| e.to_str()) == Some("webm") {
        "webm"
    } else {
        "mp4"
    };
    run_ffmpeg(
        ffmpeg,
        [
            OsStr::new("-i"),
            video.as_os_str(),
            OsStr::new("-i"),
            audio.as_os_str(),
            OsStr::new("-c"),
            OsStr::new("copy"),
            OsStr::new("-movflags"),
            OsStr::new("+faststart"),
            OsStr::new("-f"),
            OsStr::new(fmt),
            output.as_os_str(),
        ],
        cancel,
    )
    .await
}
