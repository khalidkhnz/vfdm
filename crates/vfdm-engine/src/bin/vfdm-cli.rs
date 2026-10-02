use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;
use vfdm_engine::{DownloadEvent, DownloadRequest, DownloadStatus, Engine, Progress, Settings};

#[derive(Parser)]
#[command(name = "vfdm-cli", about = "vfdm engine test driver")]
struct Cli {
    /// Engine state directory (sidecars live here)
    #[arg(long, default_value = ".vfdm-data")]
    data_dir: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Download one URL and print progress until it finishes
    Get {
        url: String,
        #[arg(short, long, default_value = ".")]
        out: PathBuf,
        #[arg(short = 'n', long, default_value_t = 8)]
        connections: u8,
        /// Pause after N seconds, wait 2s, resume (exercises the resume path)
        #[arg(long)]
        pause_after: Option<u64>,
    },
    /// List downloads known in the data dir
    List,
    /// Resume a paused/failed download by id and wait for it
    Resume { id: u64 },
}

#[tokio::main]
async fn main() -> anyhow_lite::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();

    match cli.cmd {
        Cmd::Get {
            url,
            out,
            connections,
            pause_after,
        } => {
            let mut settings = Settings::new(out);
            settings.max_connections = connections;
            let engine = Engine::new(settings, &cli.data_dir)?;
            let id = engine.add(DownloadRequest {
                url,
                max_connections: Some(connections),
                ..Default::default()
            })?;
            if let Some(secs) = pause_after {
                let e = engine.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    eprintln!("\n[cli] pausing");
                    let _ = e.pause(id);
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    eprintln!("[cli] resuming");
                    let _ = e.resume(id);
                });
            }
            wait(&engine, id).await?;
            engine.shutdown().await;
        }
        Cmd::List => {
            let engine = Engine::new(Settings::new(".".into()), &cli.data_dir)?;
            engine.load_all()?;
            for p in engine.list() {
                println!(
                    "{:>4}  {:<12} {:>10} / {:<10} {}",
                    p.id,
                    status_str(&p.status),
                    p.downloaded,
                    p.total.map(|t| t.to_string()).unwrap_or("?".into()),
                    p.filename
                );
            }
            engine.shutdown().await;
        }
        Cmd::Resume { id } => {
            let engine = Engine::new(Settings::new(".".into()), &cli.data_dir)?;
            engine.load_all()?;
            engine.resume(id)?;
            wait(&engine, id).await?;
            engine.shutdown().await;
        }
    }
    Ok(())
}

async fn wait(engine: &Engine, id: u64) -> anyhow_lite::Result<()> {
    let mut rx = engine.subscribe();
    loop {
        match rx.recv().await {
            Ok(DownloadEvent::Progress(p)) if p.id == id => print_progress(&p),
            Ok(DownloadEvent::Status { id: i, status }) if i == id => match status {
                DownloadStatus::Completed => {
                    let p = engine.get(id).unwrap();
                    println!("\ndone: {}", p.final_path.display());
                    return Ok(());
                }
                DownloadStatus::Failed(msg) => {
                    println!("\nfailed: {msg}");
                    return Err(msg.into());
                }
                DownloadStatus::Cancelled => {
                    println!("\ncancelled");
                    return Ok(());
                }
                s => eprintln!("\n[{}]", status_str(&s)),
            },
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(_) => return Ok(()),
        }
    }
}

fn print_progress(p: &Progress) {
    let total = p.total.unwrap_or(0);
    let pct = if total > 0 {
        p.downloaded as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    let bar: String = p
        .segments
        .iter()
        .map(|s| {
            let len = s.end.saturating_sub(s.start) + 1;
            if s.downloaded >= len {
                '#'
            } else if s.downloaded > 0 {
                '+'
            } else {
                '.'
            }
        })
        .collect();
    print!(
        "\r{:>6.2}%  {:>8}/s  eta {:>5}  segs[{}] {}",
        pct,
        human(p.speed_bps as u64),
        p.eta_secs.map(|e| format!("{e}s")).unwrap_or("-".into()),
        p.segments.len(),
        bar
    );
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

fn human(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1}{}", U[i])
}

fn status_str(s: &DownloadStatus) -> String {
    match s {
        DownloadStatus::Failed(_) => "failed".into(),
        other => format!("{other:?}").to_lowercase(),
    }
}

mod anyhow_lite {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
}
