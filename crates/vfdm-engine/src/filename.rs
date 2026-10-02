use percent_encoding::percent_decode_str;
use std::path::{Path, PathBuf};
use url::Url;

const MAX_LEN: usize = 200;
const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Pick a filename: explicit hint > Content-Disposition > URL path > mime > "download".
pub fn resolve(
    hint: Option<&str>,
    content_disposition: Option<&str>,
    url: &Url,
    content_type: Option<&str>,
) -> String {
    let candidate = hint
        .map(sanitize)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            content_disposition
                .and_then(from_content_disposition)
                .map(|s| sanitize(&s))
        })
        .filter(|s| !s.is_empty())
        .or_else(|| from_url(url).map(|s| sanitize(&s)))
        .filter(|s| !s.is_empty());

    match candidate {
        Some(name) => name,
        None => {
            let ext = content_type.and_then(ext_from_mime).unwrap_or("bin");
            format!("download.{ext}")
        }
    }
}

/// RFC 6266: prefers `filename*=UTF-8''...`, falls back to `filename=`.
pub fn from_content_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    let mut ext = None;
    for part in value.split(';').skip(1) {
        let part = part.trim();
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim();
        if k == "filename*" {
            let v = v.trim_matches('"');
            let v = v
                .strip_prefix("UTF-8''")
                .or_else(|| v.strip_prefix("utf-8''"))
                .unwrap_or_else(|| v.rsplit("''").next().unwrap_or(v));
            ext = Some(percent_decode_str(v).decode_utf8_lossy().into_owned());
        } else if k == "filename" {
            plain = Some(v.trim_matches('"').to_string());
        }
    }
    ext.or(plain).filter(|s| !s.trim().is_empty())
}

pub fn from_url(url: &Url) -> Option<String> {
    let seg = url.path_segments()?.rfind(|s| !s.is_empty())?;
    let decoded = percent_decode_str(seg).decode_utf8_lossy().into_owned();
    if decoded.trim().is_empty() {
        None
    } else {
        Some(decoded)
    }
}

fn ext_from_mime(ct: &str) -> Option<&'static str> {
    let ct = ct.split(';').next()?.trim().to_ascii_lowercase();
    Some(match ct.as_str() {
        "application/zip" => "zip",
        "application/pdf" => "pdf",
        "application/json" => "json",
        "application/gzip" | "application/x-gzip" => "gz",
        "application/x-tar" => "tar",
        "application/x-7z-compressed" => "7z",
        "application/x-rar-compressed" | "application/vnd.rar" => "rar",
        "application/x-msdownload" => "exe",
        "application/x-apple-diskimage" => "dmg",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "text/plain" => "txt",
        "text/html" => "html",
        _ => return None,
    })
}

/// Strip path separators, control chars, reserved names, trailing dots/spaces; cap length.
pub fn sanitize(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut out: String = base
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    out = out.trim().trim_end_matches(['.', ' ']).to_string();

    let stem_upper = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&stem_upper.as_str()) {
        out = format!("_{out}");
    }

    if out.len() > MAX_LEN {
        let (stem, ext) = split_ext(&out);
        let keep = MAX_LEN.saturating_sub(ext.len());
        let mut cut = keep;
        while !stem.is_char_boundary(cut) {
            cut -= 1;
        }
        out = format!("{}{}", &stem[..cut], ext);
    }
    out
}

/// Returns (stem, ".ext") — ext includes the dot, or is empty.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// `dir/name` unless it or its `.part` exists; then `name (1)`, `name (2)`, ...
pub fn dedupe(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !exists_or_part(&first) {
        return first;
    }
    let (stem, ext) = split_ext(name);
    for n in 1..10_000 {
        let p = dir.join(format!("{stem} ({n}){ext}"));
        if !exists_or_part(&p) {
            return p;
        }
    }
    dir.join(format!("{stem} ({}){ext}", crate::types::now_millis()))
}

fn exists_or_part(p: &Path) -> bool {
    p.exists() || part_path(p).exists()
}

pub fn part_path(final_path: &Path) -> PathBuf {
    let mut s = final_path.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition_variants() {
        assert_eq!(
            from_content_disposition(r#"attachment; filename="a b.zip""#).as_deref(),
            Some("a b.zip")
        );
        assert_eq!(
            from_content_disposition("attachment; filename*=UTF-8''na%C3%AFve.pdf").as_deref(),
            Some("naïve.pdf")
        );
        assert_eq!(
            from_content_disposition(r#"attachment; filename="x.txt"; filename*=UTF-8''y.txt"#)
                .as_deref(),
            Some("y.txt")
        );
        assert_eq!(from_content_disposition("inline"), None);
    }

    #[test]
    fn sanitize_rules() {
        assert_eq!(sanitize("../../etc/passwd"), "passwd");
        assert_eq!(sanitize("a:b?c.txt"), "a_b_c.txt");
        assert_eq!(sanitize("CON.txt"), "_CON.txt");
        assert_eq!(sanitize("trailing. . "), "trailing");
        assert!(sanitize(&"x".repeat(500)).len() <= MAX_LEN);
    }

    #[test]
    fn url_fallback() {
        let u = Url::parse("https://h.com/a/b/file%20name.iso?x=1").unwrap();
        assert_eq!(from_url(&u).as_deref(), Some("file name.iso"));
        let u = Url::parse("https://h.com/").unwrap();
        assert_eq!(from_url(&u), None);
        assert_eq!(
            resolve(None, None, &u, Some("application/zip")),
            "download.zip"
        );
    }
}
