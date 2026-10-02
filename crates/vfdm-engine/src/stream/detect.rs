use crate::types::Kind;
use url::Url;

pub fn by_url(url: &str) -> Option<Kind> {
    let u = Url::parse(url).ok()?;
    let path = u.path().to_ascii_lowercase();
    if path.ends_with(".m3u8") || path.ends_with(".m3u") {
        Some(Kind::Hls)
    } else if path.ends_with(".mpd") {
        Some(Kind::Dash)
    } else {
        None
    }
}

pub fn by_content_type(ct: &str) -> Option<Kind> {
    let ct = ct.split(';').next()?.trim().to_ascii_lowercase();
    match ct.as_str() {
        "application/vnd.apple.mpegurl"
        | "application/x-mpegurl"
        | "application/mpegurl"
        | "audio/mpegurl"
        | "audio/x-mpegurl" => Some(Kind::Hls),
        "application/dash+xml" => Some(Kind::Dash),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_detection() {
        assert_eq!(by_url("https://a/b/master.M3U8?x=1"), Some(Kind::Hls));
        assert_eq!(by_url("https://a/b/manifest.mpd"), Some(Kind::Dash));
        assert_eq!(by_url("https://a/b/file.mp4"), None);
        assert_eq!(by_url("nope"), None);
    }

    #[test]
    fn content_type_detection() {
        assert_eq!(
            by_content_type("application/vnd.apple.mpegurl; charset=utf-8"),
            Some(Kind::Hls)
        );
        assert_eq!(by_content_type("Application/X-MPEGURL"), Some(Kind::Hls));
        assert_eq!(by_content_type("application/dash+xml"), Some(Kind::Dash));
        assert_eq!(by_content_type("video/mp4"), None);
    }
}
