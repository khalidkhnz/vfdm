use crate::types::{DownloadId, DownloadMeta};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Persists one JSON sidecar per download under `<data_dir>/sidecars/`.
/// Kept out of the download directory because requests carry cookies.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(data_dir: &Path) -> io::Result<Self> {
        let dir = data_dir.join("sidecars");
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    pub fn path(&self, id: DownloadId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Atomic: write `.tmp`, then rename over the target.
    pub fn save(&self, meta: &DownloadMeta) -> io::Result<()> {
        let target = self.path(meta.id);
        let tmp = self.dir.join(format!("{}.json.tmp", meta.id));
        let json = serde_json::to_vec_pretty(meta)?;
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &target)
    }

    pub fn load(&self, id: DownloadId) -> io::Result<DownloadMeta> {
        let bytes = fs::read(self.path(id))?;
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    }

    pub fn load_all(&self) -> io::Result<Vec<DownloadMeta>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let p = entry?.path();
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match fs::read(&p)
                .and_then(|b| serde_json::from_slice::<DownloadMeta>(&b).map_err(io::Error::other))
            {
                Ok(m) => out.push(m),
                Err(e) => {
                    tracing::warn!(path = %p.display(), error = %e, "skipping unreadable sidecar")
                }
            }
        }
        out.sort_by_key(|m| m.created_at);
        Ok(out)
    }

    pub fn delete(&self, id: DownloadId) -> io::Result<()> {
        match fs::remove_file(self.path(id)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }
}
