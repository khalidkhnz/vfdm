use super::{crypto, fetch, Container, KeySpec, Plan, Resource, SegmentPlan, TrackPlan};
use crate::download::RunCtx;
use crate::error::{EngineError, Result};
use crate::types::{DownloadRequest, TrackRole, VariantInfo};
use m3u8_rs::{AlternativeMediaType, MasterPlaylist, MediaPlaylist, Playlist};
use std::collections::HashMap;
use url::Url;

pub enum HlsDoc {
    Master(MasterPlaylist),
    Media(MediaPlaylist),
}

pub fn parse(bytes: &[u8]) -> Result<HlsDoc> {
    match m3u8_rs::parse_playlist_res(bytes) {
        Ok(Playlist::MasterPlaylist(m)) => Ok(HlsDoc::Master(m)),
        Ok(Playlist::MediaPlaylist(m)) => Ok(HlsDoc::Media(m)),
        Err(e) => Err(EngineError::Other(format!("invalid m3u8: {e:?}"))),
    }
}

/// Fetch the manifest (and, for a master playlist, the chosen media playlist
/// plus any separate audio rendition) and turn it into a download plan.
pub async fn load(
    ctx: &RunCtx,
    request: &DownloadRequest,
    prior_chosen: Option<u32>,
) -> Result<Plan> {
    let url = Url::parse(&request.url).map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
    let (bytes, final_url) =
        fetch::fetch_bytes_url(ctx, &ctx.cancel, request, &Resource { url, range: None }).await?;

    match parse(&bytes)? {
        HlsDoc::Media(pl) => Ok(Plan {
            manifest_url: final_url.clone(),
            variants: Vec::new(),
            chosen: None,
            tracks: vec![plan_media(&pl, &bytes, &final_url, TrackRole::Muxed)?],
        }),
        HlsDoc::Master(mp) => {
            let variants = variant_infos(&mp);
            if variants.is_empty() {
                return Err(EngineError::Other("master playlist has no variants".into()));
            }
            let chosen = prior_chosen
                .filter(|&i| (i as usize) < variants.len())
                .unwrap_or(0);
            let v = &variants[chosen as usize];
            let media_url = final_url
                .join(&v.uri)
                .map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
            let (mb, media_final) = fetch::fetch_bytes_url(
                ctx,
                &ctx.cancel,
                request,
                &Resource {
                    url: media_url,
                    range: None,
                },
            )
            .await?;
            let HlsDoc::Media(pl) = parse(&mb)? else {
                return Err(EngineError::Other(
                    "nested master playlists are not supported".into(),
                ));
            };

            let audio_uri = v
                .audio_group
                .as_deref()
                .and_then(|g| audio_rendition(&mp, g));
            let tracks = match audio_uri {
                Some(au) => {
                    let audio_url = final_url
                        .join(&au)
                        .map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
                    let (ab, audio_final) = fetch::fetch_bytes_url(
                        ctx,
                        &ctx.cancel,
                        request,
                        &Resource {
                            url: audio_url,
                            range: None,
                        },
                    )
                    .await?;
                    let HlsDoc::Media(apl) = parse(&ab)? else {
                        return Err(EngineError::Other(
                            "audio rendition is not a media playlist".into(),
                        ));
                    };
                    vec![
                        plan_media(&pl, &mb, &media_final, TrackRole::Video)?,
                        plan_media(&apl, &ab, &audio_final, TrackRole::Audio)?,
                    ]
                }
                None => vec![plan_media(&pl, &mb, &media_final, TrackRole::Muxed)?],
            };
            Ok(Plan {
                manifest_url: final_url,
                variants,
                chosen: Some(chosen),
                tracks,
            })
        }
    }
}

/// Non-I-frame variants, best first (bandwidth, then resolution).
pub fn variant_infos(mp: &MasterPlaylist) -> Vec<VariantInfo> {
    let mut v: Vec<VariantInfo> = mp
        .variants
        .iter()
        .filter(|v| !v.is_i_frame)
        .map(|v| VariantInfo {
            uri: v.uri.clone(),
            bandwidth: v.bandwidth,
            resolution: v.resolution.map(|r| (r.width as u32, r.height as u32)),
            codecs: v.codecs.clone(),
            audio_group: v.audio.clone(),
        })
        .collect();
    let area = |v: &VariantInfo| v.resolution.map(|(w, h)| w as u64 * h as u64).unwrap_or(0);
    v.sort_by(|a, b| b.bandwidth.cmp(&a.bandwidth).then(area(b).cmp(&area(a))));
    v
}

/// URI of the audio rendition for a group, preferring DEFAULT then AUTOSELECT.
/// Renditions without a URI are muxed into the variant and need no track.
pub fn audio_rendition(mp: &MasterPlaylist, group: &str) -> Option<String> {
    mp.alternatives
        .iter()
        .filter(|a| {
            a.media_type == AlternativeMediaType::Audio && a.group_id == group && a.uri.is_some()
        })
        .max_by_key(|a| (a.default, a.autoselect))
        .and_then(|a| a.uri.clone())
}

/// Ordered segment list with absolute byte ranges, resolved keys and init maps.
pub fn plan_media(
    pl: &MediaPlaylist,
    raw: &[u8],
    base: &Url,
    role: TrackRole,
) -> Result<TrackPlan> {
    if !pl.end_list {
        return Err(EngineError::Other(
            "live stream (no EXT-X-ENDLIST) is not supported".into(),
        ));
    }
    let mut inits: Vec<Resource> = Vec::new();
    let mut segments: Vec<SegmentPlan> = Vec::new();
    // m3u8-rs drops `METHOD=NONE` tags and only reports a key on the segment
    // where the tag sits, so key state is tracked from the raw text instead.
    let keys = scan_keys(&String::from_utf8_lossy(raw));
    if keys.len() != pl.segments.len() {
        return Err(EngineError::Other(format!(
            "playlist parse mismatch: {} uris vs {} segments",
            keys.len(),
            pl.segments.len()
        )));
    }
    let mut cur_map: Option<u32> = None;
    let mut prev_end: HashMap<String, u64> = HashMap::new();
    let mut fmp4 = false;

    for (idx, seg) in pl.segments.iter().enumerate() {
        // MAP is only attached where the tag appears; it applies onward.
        if let Some(map) = &seg.map {
            let url = base
                .join(&map.uri)
                .map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
            let range = map.byte_range.as_ref().map(|br| {
                let off = br.offset.unwrap_or(0);
                (off, off + br.length - 1)
            });
            let res = Resource { url, range };
            let i = match inits.iter().position(|r| *r == res) {
                Some(i) => i,
                None => {
                    inits.push(res);
                    inits.len() - 1
                }
            };
            cur_map = Some(i as u32);
            fmp4 = true;
        }

        let url = base
            .join(&seg.uri)
            .map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
        let range = seg.byte_range.as_ref().map(|br| {
            let start = br
                .offset
                .unwrap_or_else(|| prev_end.get(url.as_str()).map(|e| e + 1).unwrap_or(0));
            let end = start + br.length.saturating_sub(1);
            prev_end.insert(url.to_string(), end);
            (start, end)
        });
        let lower = url.path().to_ascii_lowercase();
        if lower.ends_with(".m4s") || lower.ends_with(".mp4") || lower.ends_with(".m4a") {
            fmp4 = true;
        }

        let key = match &keys[idx] {
            None => None,
            Some(k) => match k.method.as_str() {
                "NONE" => None,
                "AES-128" => {
                    if k.keyformat.as_deref().is_some_and(|f| f != "identity") {
                        return Err(EngineError::Other(format!(
                            "key format {:?} (DRM) is not supported",
                            k.keyformat
                        )));
                    }
                    let kuri = k
                        .uri
                        .as_deref()
                        .ok_or_else(|| EngineError::Other("EXT-X-KEY without URI".into()))?;
                    let kurl = base
                        .join(kuri)
                        .map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
                    let iv = match &k.iv {
                        Some(h) => crypto::parse_iv(h)?,
                        None => crypto::iv_from_sequence(pl.media_sequence + idx as u64),
                    };
                    Some(KeySpec { url: kurl, iv })
                }
                m if m.starts_with("SAMPLE-AES") => {
                    return Err(EngineError::Other(
                        "stream is encrypted with SAMPLE-AES (DRM) — not supported".into(),
                    ))
                }
                m => {
                    return Err(EngineError::Other(format!(
                        "unsupported encryption method {m}"
                    )))
                }
            },
        };

        segments.push(SegmentPlan {
            res: Resource { url, range },
            key,
            init_idx: cur_map,
        });
    }

    let fingerprint = TrackPlan::fingerprint_of(&segments);
    Ok(TrackPlan {
        role,
        inits,
        segments,
        container: if fmp4 { Container::Fmp4 } else { Container::Ts },
        fingerprint,
    })
}

#[derive(Debug, Clone)]
struct RawKey {
    method: String,
    uri: Option<String>,
    iv: Option<String>,
    keyformat: Option<String>,
}

/// Key in effect for each media URI, in playlist order.
fn scan_keys(text: &str) -> Vec<Option<RawKey>> {
    let mut cur: Option<RawKey> = None;
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(attrs) = l.strip_prefix("#EXT-X-KEY:") {
            let a = parse_attrs(attrs);
            let get = |k: &str| a.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
            cur = Some(RawKey {
                method: get("METHOD")
                    .unwrap_or_else(|| "NONE".into())
                    .to_ascii_uppercase(),
                uri: get("URI"),
                iv: get("IV"),
                keyformat: get("KEYFORMAT"),
            });
        } else if !l.starts_with('#') {
            out.push(cur.clone());
        }
    }
    out
}

/// `A=1,B="x,y"` → [("A","1"),("B","x,y")]
fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let push = |item: &str, out: &mut Vec<(String, String)>| {
        if let Some((k, v)) = item.split_once('=') {
            out.push((k.trim().to_string(), v.trim().trim_matches('"').to_string()));
        }
    };
    for c in s.chars() {
        match c {
            '"' => {
                in_q = !in_q;
                cur.push(c);
            }
            ',' if !in_q => {
                push(&cur, &mut out);
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    push(&cur, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &str = "#EXTM3U
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",DEFAULT=YES,URI=\"audio/en.m3u8\"
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,AUDIO=\"aud\"
v360.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1280x720,AUDIO=\"aud\"
v720.m3u8
#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=90000,URI=\"iframes.m3u8\"
#EXT-X-STREAM-INF:BANDWIDTH=1500000,RESOLUTION=960x540
v540.m3u8
";

    const MEDIA_KEYED: &str = "#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXT-X-MEDIA-SEQUENCE:10
#EXT-X-KEY:METHOD=AES-128,URI=\"k.bin\"
#EXTINF:6.0,
s10.ts
#EXTINF:6.0,
s11.ts
#EXT-X-KEY:METHOD=AES-128,URI=\"k2.bin\",IV=0x000102030405060708090a0b0c0d0e0f
#EXTINF:6.0,
s12.ts
#EXT-X-KEY:METHOD=NONE
#EXTINF:6.0,
s13.ts
#EXT-X-ENDLIST
";

    const MEDIA_FMP4: &str = "#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:4
#EXT-X-MAP:URI=\"all.mp4\",BYTERANGE=\"700@0\"
#EXTINF:4.0,
#EXT-X-BYTERANGE:1000@700
all.mp4
#EXTINF:4.0,
#EXT-X-BYTERANGE:1000
all.mp4
#EXT-X-DISCONTINUITY
#EXT-X-MAP:URI=\"other.mp4\"
#EXTINF:4.0,
seg3.m4s
#EXT-X-ENDLIST
";

    fn base() -> Url {
        Url::parse("https://cdn.example.com/v/x/index.m3u8").unwrap()
    }

    #[test]
    fn master_variants_sorted_and_audio_group() {
        let HlsDoc::Master(mp) = parse(MASTER.as_bytes()).unwrap() else {
            panic!()
        };
        let v = variant_infos(&mp);
        assert_eq!(v.len(), 3, "i-frame variant excluded");
        assert_eq!(v[0].uri, "v720.m3u8");
        assert_eq!(v[0].resolution, Some((1280, 720)));
        assert_eq!(v[2].uri, "v360.m3u8");
        assert_eq!(
            audio_rendition(&mp, "aud").as_deref(),
            Some("audio/en.m3u8")
        );
        assert_eq!(audio_rendition(&mp, "nope"), None);
    }

    #[test]
    fn keyed_media_carries_key_forward() {
        let HlsDoc::Media(pl) = parse(MEDIA_KEYED.as_bytes()).unwrap() else {
            panic!()
        };
        let t = plan_media(&pl, MEDIA_KEYED.as_bytes(), &base(), TrackRole::Muxed).unwrap();
        assert_eq!(t.segments.len(), 4);
        assert_eq!(t.container, Container::Ts);
        let k0 = t.segments[0].key.as_ref().unwrap();
        assert_eq!(k0.url.as_str(), "https://cdn.example.com/v/x/k.bin");
        assert_eq!(k0.iv, crypto::iv_from_sequence(10));
        let k1 = t.segments[1].key.as_ref().unwrap();
        assert_eq!(
            k1.iv,
            crypto::iv_from_sequence(11),
            "key carried to next segment"
        );
        let k2 = t.segments[2].key.as_ref().unwrap();
        assert_eq!(k2.url.path(), "/v/x/k2.bin");
        assert_eq!(k2.iv[15], 0x0f);
        assert!(t.segments[3].key.is_none(), "METHOD=NONE clears");
        assert!(t.fingerprint.starts_with("4|"));
    }

    #[test]
    fn fmp4_byteranges_and_maps() {
        let HlsDoc::Media(pl) = parse(MEDIA_FMP4.as_bytes()).unwrap() else {
            panic!()
        };
        let t = plan_media(&pl, MEDIA_FMP4.as_bytes(), &base(), TrackRole::Muxed).unwrap();
        assert_eq!(t.container, Container::Fmp4);
        assert_eq!(t.inits.len(), 2);
        assert_eq!(t.inits[0].range, Some((0, 699)));
        assert_eq!(t.segments[0].res.range, Some((700, 1699)));
        assert_eq!(
            t.segments[1].res.range,
            Some((1700, 2699)),
            "implicit offset continues"
        );
        assert_eq!(t.segments[0].init_idx, Some(0));
        assert_eq!(t.segments[2].init_idx, Some(1));
        assert_eq!(t.segments[2].res.url.path(), "/v/x/seg3.m4s");
    }

    #[test]
    fn live_and_drm_rejected() {
        let live = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\na.ts\n";
        let HlsDoc::Media(pl) = parse(live.as_bytes()).unwrap() else {
            panic!()
        };
        let e = plan_media(&pl, live.as_bytes(), &base(), TrackRole::Muxed)
            .unwrap_err()
            .to_string();
        assert!(e.contains("live"), "{e}");

        let drm = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\"\n#EXTINF:6.0,\na.ts\n#EXT-X-ENDLIST\n";
        let HlsDoc::Media(pl) = parse(drm.as_bytes()).unwrap() else {
            panic!()
        };
        let e = plan_media(&pl, drm.as_bytes(), &base(), TrackRole::Muxed)
            .unwrap_err()
            .to_string();
        assert!(e.contains("SAMPLE-AES"), "{e}");
    }
}
