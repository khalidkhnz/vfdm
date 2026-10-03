//! MPEG-DASH (static MPD) → two-track plan. Hand-rolled over roxmltree:
//! only a dozen attributes matter and the `dash-mpd` crate pulls in a lot.

use super::{fetch, Container, Plan, Resource, SegmentPlan, TrackPlan};
use crate::download::RunCtx;
use crate::error::{EngineError, Result};
use crate::types::{DownloadRequest, TrackRole};
use roxmltree::{Document, Node};
use url::Url;

#[derive(Debug, Clone, Default)]
struct Template {
    media: Option<String>,
    initialization: Option<String>,
    start_number: Option<u64>,
    duration: Option<u64>,
    timescale: Option<u64>,
    timeline: Option<Vec<S>>,
}

#[derive(Debug, Clone, Copy)]
struct S {
    t: Option<u64>,
    d: u64,
    r: i64,
}

#[derive(Debug, Clone, Default)]
struct SegList {
    init: Option<(String, Option<(u64, u64)>)>,
    urls: Vec<(String, Option<(u64, u64)>)>,
}

#[derive(Debug, Clone, Default)]
struct SegBase {
    init_range: Option<(u64, u64)>,
}

#[derive(Debug, Clone)]
pub struct Representation {
    pub id: String,
    pub bandwidth: u64,
    pub width: u64,
    pub height: u64,
    pub mime: String,
    pub codecs: Option<String>,
    base: Url,
    tmpl: Template,
    list: Option<SegList>,
    sbase: Option<SegBase>,
}

#[derive(Debug, Clone)]
pub struct AdaptationSet {
    pub content_type: String,
    pub lang: Option<String>,
    pub reps: Vec<Representation>,
}

#[derive(Debug, Clone)]
pub struct MpdDoc {
    pub dynamic: bool,
    pub duration_secs: Option<f64>,
    pub sets: Vec<AdaptationSet>,
}

pub async fn load(ctx: &RunCtx, request: &DownloadRequest) -> Result<Plan> {
    let url = Url::parse(&request.url).map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
    let (bytes, final_url) =
        fetch::fetch_bytes_url(ctx, &ctx.cancel, request, &Resource { url, range: None }).await?;
    let text = String::from_utf8_lossy(&bytes);
    let doc = parse(&text, &final_url)?;
    let mut plan = plan(&doc)?;
    plan.manifest_url = final_url;
    Ok(plan)
}

fn attr<'a>(n: &Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attribute(name)
}

fn attr_u64(n: &Node, name: &str) -> Option<u64> {
    attr(n, name).and_then(|v| v.trim().parse().ok())
}

fn child<'a, 'i>(n: &Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

fn children<'a, 'i>(n: &Node<'a, 'i>, name: &str) -> Vec<Node<'a, 'i>> {
    n.children()
        .filter(|c| c.is_element() && c.tag_name().name() == name)
        .collect()
}

fn base_url(n: &Node, parent: &Url) -> Url {
    child(n, "BaseURL")
        .and_then(|b| b.text())
        .and_then(|t| parent.join(t.trim()).ok())
        .unwrap_or_else(|| parent.clone())
}

fn parse_range(s: &str) -> Option<(u64, u64)> {
    let (a, b) = s.split_once('-')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// `PT1H2M3.5S` → seconds.
pub fn parse_duration(s: &str) -> Option<f64> {
    let s = s.trim().strip_prefix('P')?;
    let (date, time) = match s.split_once('T') {
        Some((d, t)) => (d, t),
        None => (s, ""),
    };
    let mut secs = 0.0;
    let mut num = String::new();
    for c in date.chars() {
        match c {
            'D' => {
                secs += num.parse::<f64>().ok()? * 86400.0;
                num.clear();
            }
            c if c.is_ascii_digit() || c == '.' => num.push(c),
            _ => return None,
        }
    }
    num.clear();
    for c in time.chars() {
        match c {
            'H' => {
                secs += num.parse::<f64>().ok()? * 3600.0;
                num.clear();
            }
            'M' => {
                secs += num.parse::<f64>().ok()? * 60.0;
                num.clear();
            }
            'S' => {
                secs += num.parse::<f64>().ok()?;
                num.clear();
            }
            c if c.is_ascii_digit() || c == '.' => num.push(c),
            _ => return None,
        }
    }
    Some(secs)
}

fn parse_template(n: &Node) -> Template {
    let timeline = child(n, "SegmentTimeline").map(|tl| {
        children(&tl, "S")
            .iter()
            .map(|s| S {
                t: attr_u64(s, "t"),
                d: attr_u64(s, "d").unwrap_or(0),
                r: attr(s, "r").and_then(|v| v.parse().ok()).unwrap_or(0),
            })
            .collect()
    });
    Template {
        media: attr(n, "media").map(str::to_string),
        initialization: attr(n, "initialization").map(str::to_string),
        start_number: attr_u64(n, "startNumber"),
        duration: attr_u64(n, "duration"),
        timescale: attr_u64(n, "timescale"),
        timeline,
    }
}

/// Representation-level attributes override AdaptationSet-level ones.
fn merge(set: &Template, rep: &Template) -> Template {
    Template {
        media: rep.media.clone().or_else(|| set.media.clone()),
        initialization: rep
            .initialization
            .clone()
            .or_else(|| set.initialization.clone()),
        start_number: rep.start_number.or(set.start_number),
        duration: rep.duration.or(set.duration),
        timescale: rep.timescale.or(set.timescale),
        timeline: rep.timeline.clone().or_else(|| set.timeline.clone()),
    }
}

fn parse_list(n: &Node) -> SegList {
    SegList {
        init: child(n, "Initialization").map(|i| {
            (
                attr(&i, "sourceURL").unwrap_or("").to_string(),
                attr(&i, "range").and_then(parse_range),
            )
        }),
        urls: children(n, "SegmentURL")
            .iter()
            .map(|u| {
                (
                    attr(u, "media").unwrap_or("").to_string(),
                    attr(u, "mediaRange").and_then(parse_range),
                )
            })
            .collect(),
    }
}

fn parse_base(n: &Node) -> SegBase {
    SegBase {
        init_range: child(n, "Initialization")
            .and_then(|i| attr(&i, "range"))
            .and_then(parse_range),
    }
}

pub fn parse(xml: &str, manifest_url: &Url) -> Result<MpdDoc> {
    let doc = Document::parse(xml).map_err(|e| EngineError::Other(format!("invalid MPD: {e}")))?;
    let root = doc.root_element();
    if root.tag_name().name() != "MPD" {
        return Err(EngineError::Other("not an MPD document".into()));
    }
    let dynamic = attr(&root, "type").is_some_and(|t| t.eq_ignore_ascii_case("dynamic"));
    let duration_secs = attr(&root, "mediaPresentationDuration").and_then(parse_duration);
    let mpd_base = base_url(&root, manifest_url);

    let periods = children(&root, "Period");
    let Some(period) = periods.first() else {
        return Err(EngineError::Other("MPD has no Period".into()));
    };
    if periods.len() > 1 {
        return Err(EngineError::Other(
            "multi-period MPDs are not supported".into(),
        ));
    }
    let period_base = base_url(period, &mpd_base);
    let period_duration = attr(period, "duration")
        .and_then(parse_duration)
        .or(duration_secs);

    let mut sets = Vec::new();
    for set in children(period, "AdaptationSet") {
        let set_base = base_url(&set, &period_base);
        let set_mime = attr(&set, "mimeType").unwrap_or("").to_string();
        let content_type = attr(&set, "contentType")
            .map(str::to_string)
            .unwrap_or_else(|| set_mime.split('/').next().unwrap_or("").to_string());
        let set_tmpl = child(&set, "SegmentTemplate")
            .map(|t| parse_template(&t))
            .unwrap_or_default();
        let set_list = child(&set, "SegmentList").map(|l| parse_list(&l));
        let set_sbase = child(&set, "SegmentBase").map(|b| parse_base(&b));
        let mut reps = Vec::new();
        for r in children(&set, "Representation") {
            let rep_tmpl = child(&r, "SegmentTemplate")
                .map(|t| parse_template(&t))
                .unwrap_or_default();
            reps.push(Representation {
                id: attr(&r, "id").unwrap_or("").to_string(),
                bandwidth: attr_u64(&r, "bandwidth").unwrap_or(0),
                width: attr_u64(&r, "width").unwrap_or(0),
                height: attr_u64(&r, "height").unwrap_or(0),
                mime: attr(&r, "mimeType")
                    .map(str::to_string)
                    .unwrap_or_else(|| set_mime.clone()),
                codecs: attr(&r, "codecs").map(str::to_string),
                base: base_url(&r, &set_base),
                tmpl: merge(&set_tmpl, &rep_tmpl),
                list: child(&r, "SegmentList")
                    .map(|l| parse_list(&l))
                    .or_else(|| set_list.clone()),
                sbase: child(&r, "SegmentBase")
                    .map(|b| parse_base(&b))
                    .or_else(|| set_sbase.clone()),
            });
        }
        let content_type = if content_type.is_empty() {
            reps.first()
                .map(|r| r.mime.split('/').next().unwrap_or("").to_string())
                .unwrap_or_default()
        } else {
            content_type
        };
        sets.push(AdaptationSet {
            content_type,
            lang: attr(&set, "lang").map(str::to_string),
            reps,
        });
    }
    Ok(MpdDoc {
        dynamic,
        duration_secs: period_duration,
        sets,
    })
}

/// `$RepresentationID$`, `$Bandwidth$`, `$Number[%0Nd]$`, `$Time[%0Nd]$`, `$$`.
pub fn substitute(
    tpl: &str,
    id: &str,
    bandwidth: u64,
    number: Option<u64>,
    time: Option<u64>,
) -> String {
    let mut out = String::with_capacity(tpl.len() + 16);
    let mut rest = tpl;
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('$') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let token = &after[..end];
        rest = &after[end + 1..];
        if token.is_empty() {
            out.push('$');
            continue;
        }
        let (name, fmt) = match token.split_once('%') {
            Some((n, f)) => (n, Some(f)),
            None => (token, None),
        };
        let width: usize = fmt
            .and_then(|f| f.trim_start_matches('0').trim_end_matches('d').parse().ok())
            .unwrap_or(0);
        let value: Option<String> = match name {
            "RepresentationID" => Some(id.to_string()),
            "Bandwidth" => Some(bandwidth.to_string()),
            "Number" => number.map(|n| format!("{n:0width$}")),
            "Time" => time.map(|t| format!("{t:0width$}")),
            _ => None,
        };
        match value {
            Some(v) => out.push_str(&v),
            None => {
                out.push('$');
                out.push_str(token);
                out.push('$');
            }
        }
    }
    out.push_str(rest);
    out
}

fn join(base: &Url, rel: &str) -> Result<Url> {
    base.join(rel)
        .map_err(|e| EngineError::InvalidUrl(e.to_string()))
}

pub fn plan_representation(
    rep: &Representation,
    period_secs: Option<f64>,
    role: TrackRole,
) -> Result<TrackPlan> {
    let mut inits: Vec<Resource> = Vec::new();
    let mut segments: Vec<SegmentPlan> = Vec::new();
    let t = &rep.tmpl;

    if let Some(media) = &t.media {
        let timescale = t.timescale.unwrap_or(1).max(1);
        let start_number = t.start_number.unwrap_or(1);
        if let Some(init) = &t.initialization {
            inits.push(Resource {
                url: join(
                    &rep.base,
                    &substitute(init, &rep.id, rep.bandwidth, None, None),
                )?,
                range: None,
            });
        }
        let init_idx = (!inits.is_empty()).then_some(0);
        let mut push = |number: u64, time: Option<u64>| -> Result<()> {
            let u = substitute(media, &rep.id, rep.bandwidth, Some(number), time);
            segments.push(SegmentPlan {
                res: Resource {
                    url: join(&rep.base, &u)?,
                    range: None,
                },
                key: None,
                init_idx,
            });
            Ok(())
        };
        match &t.timeline {
            Some(tl) => {
                let mut time = 0u64;
                let mut number = start_number;
                for s in tl {
                    if let Some(tt) = s.t {
                        time = tt;
                    }
                    let repeats = if s.r < 0 { 0 } else { s.r as u64 };
                    for _ in 0..=repeats {
                        push(number, Some(time))?;
                        number += 1;
                        time += s.d;
                    }
                }
            }
            None => {
                let d = t.duration.ok_or_else(|| {
                    EngineError::Other("SegmentTemplate without duration or timeline".into())
                })?;
                let secs = period_secs.ok_or_else(|| {
                    EngineError::Other("MPD has no mediaPresentationDuration".into())
                })?;
                let count = ((secs * timescale as f64) / d as f64).ceil() as u64;
                for i in 0..count.max(1) {
                    push(start_number + i, None)?;
                }
            }
        }
    } else if let Some(list) = &rep.list {
        if let Some((u, range)) = &list.init {
            let url = if u.is_empty() {
                rep.base.clone()
            } else {
                join(&rep.base, u)?
            };
            inits.push(Resource { url, range: *range });
        }
        let init_idx = (!inits.is_empty()).then_some(0);
        for (u, range) in &list.urls {
            let url = if u.is_empty() {
                rep.base.clone()
            } else {
                join(&rep.base, u)?
            };
            segments.push(SegmentPlan {
                res: Resource { url, range: *range },
                key: None,
                init_idx,
            });
        }
    } else {
        // SegmentBase / plain BaseURL: the representation is one file.
        let init_range = rep.sbase.as_ref().and_then(|b| b.init_range);
        match init_range {
            Some((s, e)) => {
                inits.push(Resource {
                    url: rep.base.clone(),
                    range: Some((s, e)),
                });
                segments.push(SegmentPlan {
                    res: Resource {
                        url: rep.base.clone(),
                        range: Some((e + 1, u64::MAX)),
                    },
                    key: None,
                    init_idx: Some(0),
                });
            }
            None => segments.push(SegmentPlan {
                res: Resource {
                    url: rep.base.clone(),
                    range: None,
                },
                key: None,
                init_idx: None,
            }),
        }
    }

    if segments.is_empty() {
        return Err(EngineError::Other("representation has no segments".into()));
    }
    let container = if rep.mime.contains("webm") {
        Container::Webm
    } else {
        Container::Fmp4
    };
    let fingerprint = TrackPlan::fingerprint_of(&segments);
    Ok(TrackPlan {
        role,
        inits,
        segments,
        container,
        fingerprint,
    })
}

fn best_rep(set: &AdaptationSet) -> Option<&Representation> {
    set.reps.iter().max_by(|a, b| {
        a.bandwidth
            .cmp(&b.bandwidth)
            .then((a.width * a.height).cmp(&(b.width * b.height)))
    })
}

pub fn plan(doc: &MpdDoc) -> Result<Plan> {
    if doc.dynamic {
        return Err(EngineError::Other(
            "live (dynamic) DASH streams are not supported".into(),
        ));
    }
    let video_set = doc.sets.iter().find(|s| s.content_type == "video");
    let audio_set = {
        let audios: Vec<&AdaptationSet> = doc
            .sets
            .iter()
            .filter(|s| s.content_type == "audio")
            .collect();
        audios
            .iter()
            .find(|s| {
                s.lang
                    .as_deref()
                    .is_some_and(|l| l.to_ascii_lowercase().starts_with("en"))
            })
            .or(audios.first())
            .copied()
    };
    let video = video_set.and_then(best_rep);
    let audio = audio_set.and_then(best_rep);

    let tracks = match (video, audio) {
        (Some(v), Some(a)) => vec![
            plan_representation(v, doc.duration_secs, TrackRole::Video)?,
            plan_representation(a, doc.duration_secs, TrackRole::Audio)?,
        ],
        (Some(v), None) => vec![plan_representation(v, doc.duration_secs, TrackRole::Muxed)?],
        (None, Some(a)) => vec![plan_representation(a, doc.duration_secs, TrackRole::Audio)?],
        (None, None) => {
            return Err(EngineError::Other(
                "MPD has no video or audio representations".into(),
            ))
        }
    };
    Ok(Plan {
        manifest_url: Url::parse("http://invalid/").expect("placeholder"),
        variants: Vec::new(),
        chosen: None,
        tracks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn murl() -> Url {
        Url::parse("https://cdn.example.com/v/manifest.mpd").unwrap()
    }

    const TEMPLATE_TIMELINE: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT0H0M12.0S">
  <BaseURL>media/</BaseURL>
  <Period>
    <AdaptationSet contentType="video" mimeType="video/mp4">
      <SegmentTemplate timescale="1000" media="$RepresentationID$/s-$Number%05d$.m4s" initialization="$RepresentationID$/init.mp4" startNumber="1">
        <SegmentTimeline><S t="0" d="4000" r="1"/><S d="4000"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v1" bandwidth="500000" width="640" height="360"/>
      <Representation id="v2" bandwidth="2000000" width="1280" height="720"/>
    </AdaptationSet>
    <AdaptationSet contentType="audio" mimeType="audio/mp4" lang="de">
      <SegmentTemplate timescale="48000" media="a/$Bandwidth$/$Time$.m4s" initialization="a/init.mp4">
        <SegmentTimeline><S t="0" d="192000" r="2"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="a1" bandwidth="128000"/>
    </AdaptationSet>
    <AdaptationSet contentType="audio" mimeType="audio/mp4" lang="en">
      <SegmentTemplate media="en/$Number$.m4s" duration="4" startNumber="0" initialization="en/init.mp4"/>
      <Representation id="a2" bandwidth="96000"/>
    </AdaptationSet>
  </Period>
</MPD>"#;

    #[test]
    fn template_timeline_and_duration() {
        let doc = parse(TEMPLATE_TIMELINE, &murl()).unwrap();
        assert!(!doc.dynamic);
        assert_eq!(doc.duration_secs, Some(12.0));
        let p = plan(&doc).unwrap();
        assert_eq!(p.tracks.len(), 2);
        let v = &p.tracks[0];
        assert_eq!(v.role, TrackRole::Video);
        assert_eq!(
            v.inits[0].url.as_str(),
            "https://cdn.example.com/v/media/v2/init.mp4",
            "best bandwidth + BaseURL chain"
        );
        let urls: Vec<&str> = v.segments.iter().map(|s| s.res.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://cdn.example.com/v/media/v2/s-00001.m4s",
                "https://cdn.example.com/v/media/v2/s-00002.m4s",
                "https://cdn.example.com/v/media/v2/s-00003.m4s"
            ]
        );
        let a = &p.tracks[1];
        assert_eq!(a.role, TrackRole::Audio);
        let urls: Vec<&str> = a.segments.iter().map(|s| s.res.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://cdn.example.com/v/media/en/0.m4s",
                "https://cdn.example.com/v/media/en/1.m4s",
                "https://cdn.example.com/v/media/en/2.m4s"
            ],
            "english preferred; duration-based count = ceil(12/4)"
        );
        assert_eq!(v.container, Container::Fmp4);
    }

    #[test]
    fn time_template_expands_timeline() {
        let doc = parse(TEMPLATE_TIMELINE, &murl()).unwrap();
        let de = doc
            .sets
            .iter()
            .find(|s| s.lang.as_deref() == Some("de"))
            .unwrap();
        let t = plan_representation(&de.reps[0], doc.duration_secs, TrackRole::Audio).unwrap();
        let urls: Vec<&str> = t.segments.iter().map(|s| s.res.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://cdn.example.com/v/media/a/128000/0.m4s",
                "https://cdn.example.com/v/media/a/128000/192000.m4s",
                "https://cdn.example.com/v/media/a/128000/384000.m4s"
            ]
        );
    }

    const LIST_AND_BASE: &str = r#"<MPD type="static" mediaPresentationDuration="PT10S">
  <Period>
    <AdaptationSet mimeType="video/webm">
      <Representation id="w" bandwidth="100">
        <BaseURL>https://other.example.org/w.webm</BaseURL>
        <SegmentBase indexRange="700-1000"><Initialization range="0-699"/></SegmentBase>
      </Representation>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4">
      <Representation id="l" bandwidth="64">
        <SegmentList>
          <Initialization sourceURL="l/init.mp4"/>
          <SegmentURL media="l/1.m4s"/><SegmentURL media="l/2.m4s" mediaRange="0-99"/>
        </SegmentList>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;

    #[test]
    fn segment_base_and_list() {
        let doc = parse(LIST_AND_BASE, &murl()).unwrap();
        let p = plan(&doc).unwrap();
        let v = &p.tracks[0];
        assert_eq!(v.container, Container::Webm);
        assert_eq!(v.inits[0].range, Some((0, 699)));
        assert_eq!(v.segments[0].res.range, Some((700, u64::MAX)));
        assert_eq!(
            v.segments[0].res.url.as_str(),
            "https://other.example.org/w.webm"
        );
        let a = &p.tracks[1];
        assert_eq!(
            a.inits[0].url.as_str(),
            "https://cdn.example.com/v/l/init.mp4"
        );
        assert_eq!(a.segments[1].res.range, Some((0, 99)));
    }

    #[test]
    fn rejects_dynamic_and_parses_durations() {
        let e = parse(r#"<MPD type="dynamic"><Period/></MPD>"#, &murl()).map(|d| plan(&d));
        assert!(matches!(e, Ok(Err(ref err)) if err.to_string().contains("live")));
        assert_eq!(parse_duration("PT1H2M3.5S"), Some(3723.5));
        assert_eq!(parse_duration("P1DT1S"), Some(86401.0));
        assert_eq!(parse_duration("PT30S"), Some(30.0));
        assert_eq!(parse_duration("garbage"), None);
    }

    #[test]
    fn substitution() {
        assert_eq!(
            substitute(
                "$RepresentationID$/$Number%05d$.m4s?b=$Bandwidth$$$",
                "v",
                9,
                Some(7),
                None
            ),
            "v/00007.m4s?b=9$"
        );
        assert_eq!(substitute("$Time$", "v", 9, None, Some(42)), "42");
        assert_eq!(substitute("$Unknown$", "v", 9, None, None), "$Unknown$");
    }
}
