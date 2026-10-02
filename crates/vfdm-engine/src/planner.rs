use crate::types::Segment;

pub const MIN_SEGMENT: u64 = 256 * 1024;
pub const SINGLE_SEGMENT_BELOW: u64 = 1024 * 1024;
pub const MIN_STEAL: u64 = 1024 * 1024;
pub const MAX_CONNECTIONS: u8 = 16;

pub fn clamp_connections(n: u8) -> u8 {
    n.clamp(1, MAX_CONNECTIONS)
}

pub fn initial_split(total: u64, max_connections: u8) -> Vec<Segment> {
    if total == 0 {
        return Vec::new();
    }
    let mut n = clamp_connections(max_connections) as u64;
    if total < SINGLE_SEGMENT_BELOW {
        n = 1;
    } else {
        n = n.min(total / MIN_SEGMENT).max(1);
    }
    let size = total / n;
    (0..n)
        .map(|i| {
            let start = i * size;
            let end = if i == n - 1 {
                total - 1
            } else {
                start + size - 1
            };
            Segment {
                id: i as u32,
                start,
                end,
                downloaded: 0,
            }
        })
        .collect()
}

/// Work stealing: split the largest remaining tail in half and hand the back
/// half to a new segment. The victim's `end` shrinks; its worker clamps on the
/// next chunk. Returns the new segment, or None if nothing is worth stealing.
pub fn steal(segments: &mut Vec<Segment>) -> Option<Segment> {
    let victim_idx = segments
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.is_done())
        .max_by_key(|(_, s)| s.remaining())
        .map(|(i, _)| i)?;
    let victim = segments[victim_idx];
    let remaining = victim.remaining();
    if remaining < 2 * MIN_STEAL {
        return None;
    }
    let mid = victim.next() + remaining / 2;
    let new = Segment {
        id: segments.len() as u32,
        start: mid,
        end: victim.end,
        downloaded: 0,
    };
    segments[victim_idx].end = mid - 1;
    segments.push(new);
    Some(new)
}

/// Segments that still need a worker: not done and not currently assigned.
pub fn unassigned<'a>(
    segments: &'a [Segment],
    assigned: &'a [u32],
) -> impl Iterator<Item = Segment> + 'a {
    segments
        .iter()
        .copied()
        .filter(move |s| !s.is_done() && !assigned.contains(&s.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covers_exactly(segs: &[Segment], total: u64) {
        let mut v: Vec<_> = segs.iter().filter(|s| !s.is_empty()).copied().collect();
        v.sort_by_key(|s| s.start);
        assert_eq!(v[0].start, 0);
        for w in v.windows(2) {
            assert_eq!(w[0].end + 1, w[1].start, "gap/overlap at {:?}", w);
        }
        assert_eq!(v.last().unwrap().end, total - 1);
    }

    #[test]
    fn split_covers_range() {
        for total in [1u64 << 20, 10 << 20, 123_456_789] {
            for n in [1u8, 3, 8, 16, 40] {
                let s = initial_split(total, n);
                assert!(s.len() <= 16);
                covers_exactly(&s, total);
            }
        }
    }

    #[test]
    fn small_file_single_segment() {
        assert_eq!(initial_split(100, 8).len(), 1);
        assert!(initial_split(0, 8).is_empty());
    }

    #[test]
    fn steal_keeps_coverage() {
        let total = 64u64 << 20;
        let mut s = initial_split(total, 2);
        s[0].downloaded = 1 << 20;
        let mut stolen = 0;
        while steal(&mut s).is_some() {
            stolen += 1;
            covers_exactly(&s, total);
        }
        assert!(stolen >= 4);
        let mut tiny = vec![Segment {
            id: 0,
            start: 0,
            end: MIN_STEAL,
            downloaded: 0,
        }];
        assert!(steal(&mut tiny).is_none());
    }
}
