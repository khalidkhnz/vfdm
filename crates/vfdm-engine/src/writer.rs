use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Create/truncate the `.part` and reserve its full size. Sparse on APFS/NTFS.
pub fn preallocate(path: &Path, len: u64) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    f.set_len(len)?;
    Ok(f)
}

pub fn open_for_write(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).open(path)
}

/// Positional write that never touches a shared cursor, so N workers can
/// write into one file concurrently.
#[cfg(unix)]
pub fn write_at(file: &File, offset: u64, buf: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

#[cfg(windows)]
pub fn write_at(file: &File, offset: u64, buf: &[u8]) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut written = 0usize;
    while written < buf.len() {
        let n = file.seek_write(&buf[written..], offset + written as u64)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "seek_write returned 0",
            ));
        }
        written += n;
    }
    Ok(())
}
