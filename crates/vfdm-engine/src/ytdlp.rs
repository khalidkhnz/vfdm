//! yt-dlp child-process runner. The job is an ordinary queue row: progress is
//! mapped onto `segments[0]`, pause kills the process (yt-dlp resumes its own
//! `.part` files with `--continue`), cancel removes the per-job temp dir.

use crate::download::{self, RunCtx, RunResult};
use crate::types::*;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

const PROGRESS_TPL: &str = "download:vfdm|%(progress.status)s|%(progress.downloaded_bytes|0)s|%(progress.total_bytes|0)s|%(progress.total_bytes_estimate|0)s|%(progress.filename)s";
const POSTPROCESS_TPL: &str = "postprocess:vfdm-pp|%(progress.status)s";
const FINAL_TPL: &str = "after_move:vfdm-final|%(filepath)s";
const OUTPUT_TPL: &str = "%(title).200B [%(id)s].%(ext)s";

#[derive(Debug, PartialEq)]
pub enum Line {
    Progress {
        status: String,
        downloaded: u64,
        total: u64,
    },
    PostProcess,
    Final(PathBuf),
    Other(String),
}

pub fn parse_line(line: &str) -> Line {
    let l = line.trim();
    if let Some(rest) = l.strip_prefix("vfdm-final|") {
        return Line::Final(PathBuf::from(rest));
    }
    if l.starts_with("vfdm-pp|") {
        return Line::PostProcess;
    }
    if let Some(rest) = l.strip_prefix("vfdm|") {
        let mut it = rest.splitn(5, '|');
        let status = it.next().unwrap_or("").to_string();
        let num =
            |s: Option<&str>| s.and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0) as u64;
        let downloaded = num(it.next());
        let total = num(it.next());
        let est = num(it.next());
        return Line::Progress {
            status,
            downloaded,
            total: if total > 0 { total } else { est },
        };
    }
    Line::Other(l.to_string())
}

pub fn build_command(
    bin: &Path,
    req: &DownloadRequest,
    dest: &Path,
    temp: &Path,
    ffmpeg: Option<&Path>,
    js_runtime: Option<&Path>,
) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args([
        "--newline",
        "--no-colors",
        "--progress",
        "--no-simulate",
        "--no-playlist",
        "--continue",
    ])
    .arg("--progress-template")
    .arg(PROGRESS_TPL)
    .arg("--progress-template")
    .arg(POSTPROCESS_TPL)
    .arg("--print")
    .arg(FINAL_TPL)
    .arg("-o")
    .arg(OUTPUT_TPL)
    .arg("-P")
    .arg(format!("home:{}", dest.display()))
    .arg("-P")
    .arg(format!("temp:{}", temp.display()));
    if let Some(ff) = ffmpeg {
        cmd.arg("--ffmpeg-location").arg(ff);
    }
    if let Some(js) = js_runtime {
        let name = js.file_stem().and_then(|s| s.to_str()).unwrap_or("node");
        let kind = if name.starts_with("deno") {
            "deno"
        } else if name.starts_with("bun") {
            "bun"
        } else {
            "node"
        };
        cmd.arg("--js-runtimes")
            .arg(format!("{kind}:{}", js.display()));
    }
    if let Some(r) = &req.referrer {
        cmd.arg("--referer").arg(r);
    }
    if let Some(ua) = &req.user_agent {
        cmd.arg("--user-agent").arg(ua);
    }
    if let Some(c) = req.cookies.as_deref().filter(|c| !c.is_empty()) {
        cmd.arg("--add-headers").arg(format!("Cookie:{c}"));
    }
    for (k, v) in &req.headers {
        cmd.arg("--add-headers").arg(format!("{k}:{v}"));
    }
    cmd.arg("--").arg(&req.url);
    cmd.env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

pub async fn run(ctx: Arc<RunCtx>) -> RunResult {
    let Some(bin) = ctx.ytdlp_path.clone() else {
        return RunResult::Failed("yt-dlp is not installed — Settings → Tools → Install".into());
    };
    ctx.set_status(DownloadStatus::Probing);

    let (request, fresh) = {
        let m = ctx.lock_meta();
        (m.request.clone(), m.part_path.as_os_str().is_empty())
    };
    let dest = request
        .dest_dir
        .clone()
        .unwrap_or_else(|| ctx.default_dir.clone());
    let temp = ctx.data_dir.join("ytdlp").join(ctx.id.to_string());
    if let Err(e) = std::fs::create_dir_all(&dest).and_then(|_| std::fs::create_dir_all(&temp)) {
        return RunResult::Failed(e.to_string());
    }
    {
        let mut m = ctx.lock_meta();
        m.part_path = temp.clone();
        m.resumable = true;
        if fresh {
            m.segments = vec![Segment {
                id: 0,
                start: 0,
                end: u64::MAX,
                downloaded: 0,
            }];
            m.total = None;
            m.total_is_estimate = true;
        }
        let _ = ctx.store.save(&m);
    }

    let mut child = match build_command(
        &bin,
        &request,
        &dest,
        &temp,
        ctx.ffmpeg_path.as_deref(),
        ctx.js_runtime.as_deref(),
    )
    .spawn()
    {
        Ok(c) => c,
        Err(e) => return RunResult::Failed(format!("cannot start yt-dlp: {e}")),
    };
    ctx.set_status(DownloadStatus::Downloading);

    // --print goes to stdout; in the implied quiet mode progress goes to stderr.
    let (tx, mut rx) = mpsc::channel::<String>(256);
    for reader in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                if tx.send(l).await.is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);

    let mut base = 0u64;
    let mut last_downloaded = 0u64;
    let mut final_path: Option<PathBuf> = None;
    let mut tail: VecDeque<String> = VecDeque::with_capacity(20);
    let mut processing = false;

    let status = loop {
        tokio::select! {
            _ = ctx.cancel.cancelled() => {
                let _ = child.kill().await;
                let _ = ctx.store.save(&ctx.lock_meta());
                return RunResult::Interrupted;
            }
            line = rx.recv() => match line {
                Some(l) => match parse_line(&l) {
                    Line::Progress { status, downloaded, total } => {
                        // yt-dlp downloads formats one after another; a drop means a new file started.
                        if downloaded < last_downloaded {
                            base += last_downloaded;
                        }
                        last_downloaded = downloaded;
                        let mut m = ctx.lock_meta();
                        if let Some(s) = m.segments.first_mut() {
                            s.downloaded = base + downloaded;
                        }
                        if total > 0 {
                            m.total = Some(base + total);
                            m.total_is_estimate = true;
                        }
                        if status == "finished" {
                            base += downloaded;
                            last_downloaded = 0;
                        }
                    }
                    Line::PostProcess => {
                        if !processing {
                            processing = true;
                            ctx.set_status(DownloadStatus::Processing);
                        }
                    }
                    Line::Final(p) => final_path = Some(p),
                    Line::Other(o) => {
                        if !o.is_empty() {
                            if tail.len() == 20 {
                                tail.pop_front();
                            }
                            tail.push_back(o);
                        }
                    }
                },
                None => break child.wait().await,
            }
        }
    };

    let status = match status {
        Ok(s) => s,
        Err(e) => return RunResult::Failed(format!("yt-dlp wait failed: {e}")),
    };
    if !status.success() {
        let msg = tail
            .iter()
            .rev()
            .find(|l| l.contains("ERROR"))
            .or_else(|| tail.back())
            .cloned()
            .unwrap_or_else(|| format!("yt-dlp exited with {status}"));
        return RunResult::Failed(msg);
    }
    let Some(path) = final_path else {
        return RunResult::Failed("yt-dlp finished without reporting an output file".into());
    };
    {
        let mut m = ctx.lock_meta();
        m.final_path = path.clone();
        m.filename = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size = std::fs::metadata(&path).map(|md| md.len()).ok();
        if let Some(sz) = size {
            if let Some(s) = m.segments.first_mut() {
                s.downloaded = sz;
                s.end = sz.saturating_sub(1);
            }
            m.total = Some(sz);
        }
        m.total_is_estimate = false;
        m.part_path = PathBuf::new();
    }
    let _ = std::fs::remove_dir_all(&temp);
    download::mark_completed(&ctx);
    RunResult::Completed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_template_lines() {
        assert_eq!(
            parse_line("vfdm|downloading|1024|4096|0|/tmp/x.f137.mp4"),
            Line::Progress {
                status: "downloading".into(),
                downloaded: 1024,
                total: 4096
            }
        );
        assert_eq!(
            parse_line("vfdm|downloading|10.0|0|500.5|a|b.mp4"),
            Line::Progress {
                status: "downloading".into(),
                downloaded: 10,
                total: 500
            },
            "estimate used when total is 0; filename may contain pipes"
        );
        assert_eq!(
            parse_line("vfdm-final|/dl/Video [abc].mp4"),
            Line::Final("/dl/Video [abc].mp4".into())
        );
        assert_eq!(parse_line("vfdm-pp|started"), Line::PostProcess);
        assert_eq!(parse_line("ERROR: boom"), Line::Other("ERROR: boom".into()));
    }

    #[test]
    fn command_shape() {
        let req = DownloadRequest {
            url: "https://youtu.be/x".into(),
            cookies: Some("a=b".into()),
            referrer: Some("https://youtube.com".into()),
            ..Default::default()
        };
        let cmd = build_command(
            Path::new("/usr/bin/yt-dlp"),
            &req,
            Path::new("/dl"),
            Path::new("/tmp/j"),
            Some(Path::new("/usr/bin/ffmpeg")),
            Some(Path::new("/usr/bin/node")),
        );
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"--no-playlist".to_string()));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--js-runtimes" && w[1] == "node:/usr/bin/node"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--add-headers" && w[1] == "Cookie:a=b"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "-P" && w[1] == "temp:/tmp/j"));
        assert_eq!(args.last().map(String::as_str), Some("https://youtu.be/x"));
    }
}
