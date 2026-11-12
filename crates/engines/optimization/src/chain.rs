//! A board's own wiring, which its program stores a segment at a time, as
//! tracks to pull tight: segments meeting end to end where nothing else
//! joins the net are one track, as pull-tight only straightens a track's
//! corners and keeps its ends where they are.

use std::collections::HashMap;
use traxide_contracts::geometry::Point;
use traxide_contracts::{Design, LayerId, NetId, Track, Via};

/// `segments`, tracks of two points, joined end to end into tracks wherever
/// exactly two of one net, layer and width meet and nothing else of their
/// net is there: no end of another of them or of `others`, no via in
/// `vias`, no pad. A corner there may move; a pad under it would be left.
pub(crate) fn chained(
    design: &Design,
    segments: &[Track],
    others: &[Track],
    vias: &[Via],
) -> Vec<Track> {
    let tracks: Vec<&Track> = segments.iter().chain(others).collect();
    let mut ends: HashMap<(NetId, LayerId, Point), Vec<usize>> = HashMap::new();
    for (i, track) in tracks.iter().enumerate() {
        for &at in [track.path.first(), track.path.last()]
            .into_iter()
            .flatten()
        {
            ends.entry((track.net, track.layer, at))
                .or_default()
                .push(i);
        }
    }
    let is_segment = |i: usize| i < segments.len();
    let on_a_pad = |net: NetId, layer: LayerId, at: Point| {
        design.nets[net.index()].pads.iter().any(|&pad| {
            (design.pad(pad).copper.iter())
                .any(|c| c.layer == layer && c.shape.nearest_point(at) == at)
        })
    };
    let has_a_via = |net: NetId, at: Point| vias.iter().any(|v| v.net == net && v.position == at);
    // The other segment that track `i` runs on into at `at`, if the two may
    // be one track there.
    let onward = |i: usize, at: Point| -> Option<usize> {
        let track = &tracks[i];
        let there = &ends[&(track.net, track.layer, at)];
        let [a, b] = there.as_slice() else {
            return None;
        };
        let other = if *a == i { *b } else { *a };
        (other != i
            && is_segment(other)
            && tracks[other].width == track.width
            && !has_a_via(track.net, at)
            && !on_a_pad(track.net, track.layer, at))
        .then_some(other)
    };
    let mut taken = vec![false; tracks.len()];
    let mut chained = Vec::new();
    for start in 0..segments.len() {
        if taken[start] {
            continue;
        }
        taken[start] = true;
        let mut path = tracks[start].path.clone();
        // On from the last point, then back from the first.
        for forwards in [true, false] {
            let mut current = start;
            loop {
                let at = if forwards {
                    path[path.len() - 1]
                } else {
                    path[0]
                };
                let Some(next) = onward(current, at).filter(|&n| !taken[n]) else {
                    break;
                };
                taken[next] = true;
                let segment = &tracks[next].path;
                let far = if segment[0] == at {
                    segment[1]
                } else {
                    segment[0]
                };
                if forwards {
                    path.push(far);
                } else {
                    path.insert(0, far);
                }
                current = next;
            }
        }
        chained.push(Track {
            path,
            ..segments[start].clone()
        });
    }
    chained
}
