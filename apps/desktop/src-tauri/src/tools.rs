//! External tools the engine can use: ffmpeg (remux/mux), yt-dlp (site
//! extractors) and a JS runtime (yt-dlp needs one for YouTube). The app
//! locates them; the engine only receives resolved paths.

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use tokio::process::Command;
use vfdm_engine::{DownloadEvent, DownloadRequest, DownloadStatus, Kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolName {
    Ffmpeg,
    Ytdlp,
    Node,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolSource {
    Override,
    Bundled,
    Path,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStatus {
    pub name: ToolName,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub source: ToolSource,
    pub installable: bool,
    pub hint: Option<String>,
}

impl ToolName {
    fn bin(self) -> &'static str {
        match (self, cfg!(windows)) {
            (Self::Ffmpeg, false) => "ffmpeg",
            (Self::Ffmpeg, true) => "ffmpeg.exe",
            (Self::Ytdlp, false) => "yt-dlp",
            (Self::Ytdlp, true) => "yt-dlp.exe",
            (Self::Node, false) => "node",
            (Self::Node, true) => "node.exe",
        }
    }

    fn alternates(self) -> &'static [&'static str] {
        match self {
            Self::Node => &["deno", "bun"],
            _ => &[],
        }
    }

    fn version_args(self) -> &'static [&'static str] {
        match self {
            Self::Ffmpeg => &["-version"],
            Self::Ytdlp | Self::Node => &["--version"],
        }
    }

    pub fn download_url(self) -> Option<&'static str> {
        match (self, std::env::consts::OS) {
            (Self::Ytdlp, "macos") => Some("https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos"),
            (Self::Ytdlp, "windows") => Some("https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"),
            (Self::Ffmpeg, "macos") => Some("https://evermeet.cx/ffmpeg/getrelease/zip"),
            (Self::Ffmpeg, "windows") => {
                Some("https://github.com/BtbN/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-gpl.zip")
            }
            _ => None,
        }
    }
}

pub fn tools_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("tools")
}

fn is_exec(p: &Path) -> bool {
    p.is_file()
}

/// Override → app tools dir → PATH → common Homebrew/local prefixes.
pub fn locate(
    data_dir: &Path,
    override_: Option<&Path>,
    name: ToolName,
) -> Option<(PathBuf, ToolSource)> {
    if let Some(o) = override_ {
        if is_exec(o) {
            return Some((o.to_path_buf(), ToolSource::Override));
        }
    }
    let bundled = tools_dir(data_dir).join(name.bin());
    if is_exec(&bundled) {
        return Some((bundled, ToolSource::Bundled));
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"] {
        dirs.push(PathBuf::from(extra));
    }
    for candidate in std::iter::once(name.bin()).chain(name.alternates().iter().copied()) {
        for d in &dirs {
            let p = d.join(candidate);
            if is_exec(&p) {
                return Some((p, ToolSource::Path));
            }
        }
    }
    None
}

pub async fn version(path: &Path, name: ToolName) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.args(name.version_args());
    #[cfg(windows)]
    {
        cmd.creation_flags(0x0800_0000);
    }
    let out = tokio::time::timeout(std::time::Duration::from_secs(10), cmd.output())
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(if out.stdout.is_empty() {
        &out.stderr
    } else {
        &out.stdout
    });
    let first = text.lines().next()?.trim();
    Some(match name {
        ToolName::Ffmpeg => first
            .strip_prefix("ffmpeg version ")
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or(first)
            .to_string(),
        _ => first.trim_start_matches('v').to_string(),
    })
}

pub async fn resolve_one(data_dir: &Path, override_: Option<&Path>, name: ToolName) -> ToolStatus {
    let found = locate(data_dir, override_, name);
    let (path, source) = match found {
        Some((p, s)) => (Some(p), s),
        None => (None, ToolSource::Missing),
    };
    let version = match &path {
        Some(p) => version(p, name).await,
        None => None,
    };
    let mut hint = None;
    if name == ToolName::Node && path.is_none() {
        hint =
            Some("yt-dlp needs Node.js or Deno for YouTube. Install Node from nodejs.org.".into());
    }
    if name == ToolName::Ffmpeg
        && cfg!(all(target_os = "macos", target_arch = "aarch64"))
        && source == ToolSource::Bundled
        && !Path::new("/Library/Apple/usr/share/rosetta").exists()
    {
        hint = Some(
            "The downloaded ffmpeg is x86_64; install Rosetta: softwareupdate --install-rosetta"
                .into(),
        );
    }
    ToolStatus {
        name,
        path,
        version,
        source,
        installable: name.download_url().is_some(),
        hint,
    }
}

pub async fn resolve_all(
    data_dir: &Path,
    ffmpeg_override: Option<&Path>,
    ytdlp_override: Option<&Path>,
) -> Vec<ToolStatus> {
    vec![
        resolve_one(data_dir, ffmpeg_override, ToolName::Ffmpeg).await,
        resolve_one(data_dir, ytdlp_override, ToolName::Ytdlp).await,
        resolve_one(data_dir, None, ToolName::Node).await,
    ]
}

/// Downloads the tool through the engine itself, then unpacks/chmods it.
pub async fn install(app: &AppHandle, name: ToolName) -> Result<ToolStatus, String> {
    let state = app.state::<AppState>();
    let url = name
        .download_url()
        .ok_or("no download available for this platform")?;
    let dir = tools_dir(&state.data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let archive_name = match name {
        ToolName::Ytdlp => "yt-dlp.download",
        ToolName::Ffmpeg => "ffmpeg.zip",
        ToolName::Node => return Err("install Node.js from nodejs.org".into()),
    };
    let archive = dir.join(archive_name);
    let _ = std::fs::remove_file(&archive);

    let mut rx = state.engine.subscribe();
    let id = state
        .engine
        .add(DownloadRequest {
            url: url.into(),
            filename: Some(archive_name.into()),
            dest_dir: Some(dir.clone()),
            kind: Kind::File,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;

    let outcome = loop {
        match rx.recv().await {
            Ok(DownloadEvent::Status { id: i, status }) if i == id => match status {
                DownloadStatus::Completed => break Ok(()),
                DownloadStatus::Failed(m) => break Err(m),
                DownloadStatus::Cancelled => break Err("cancelled".into()),
                _ => {}
            },
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(_) => break Err("engine stopped".into()),
        }
    };
    let final_path = state
        .engine
        .get(id)
        .map(|p| p.final_path)
        .unwrap_or(archive.clone());
    let _ = state.engine.remove(id, false).await;
    outcome?;

    let bin = dir.join(name.bin());
    match name {
        ToolName::Ytdlp => {
            std::fs::rename(&final_path, &bin).map_err(|e| e.to_string())?;
        }
        ToolName::Ffmpeg => {
            extract_ffmpeg(&final_path, &bin)?;
            let _ = std::fs::remove_file(&final_path);
        }
        ToolName::Node => unreachable!(),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(target_os = "macos")]
    {
        // Files we write are not quarantined, but be explicit for the unzip case.
        let _ = std::process::Command::new("xattr")
            .args(["-d", "com.apple.quarantine"])
            .arg(&bin)
            .output();
    }

    state.refresh_tools().await;
    let tools = state
        .tools
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    tools
        .into_iter()
        .find(|t| t.name == name)
        .ok_or_else(|| "tool status missing".into())
}

/// Pulls the first entry named `ffmpeg`/`ffmpeg.exe` (any folder depth) out of the zip.
fn extract_ffmpeg(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let f = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    let want = ToolName::Ffmpeg.bin();
    let idx = (0..z.len())
        .find(|&i| {
            z.by_index(i)
                .ok()
                .map(|e| e.name().rsplit('/').next() == Some(want) && !e.is_dir())
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("{want} not found in archive"))?;
    let mut entry = z.by_index(idx).map_err(|e| e.to_string())?;
    let mut out = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locate_prefers_override_then_bundled_then_path() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(tools_dir(&data)).unwrap();
        assert!(locate(&data, None, ToolName::Ytdlp)
            .map(|(_, s)| s != ToolSource::Bundled)
            .unwrap_or(true));

        let bundled = tools_dir(&data).join(ToolName::Ytdlp.bin());
        std::fs::write(&bundled, b"#!/bin/sh\n").unwrap();
        assert_eq!(
            locate(&data, None, ToolName::Ytdlp).unwrap().1,
            ToolSource::Bundled
        );

        let ovr = tmp.path().join("custom-yt-dlp");
        std::fs::write(&ovr, b"#!/bin/sh\n").unwrap();
        assert_eq!(
            locate(&data, Some(&ovr), ToolName::Ytdlp).unwrap(),
            (ovr.clone(), ToolSource::Override)
        );
        assert_eq!(
            locate(&data, Some(Path::new("/nope/x")), ToolName::Ytdlp)
                .unwrap()
                .1,
            ToolSource::Bundled
        );
    }
}
