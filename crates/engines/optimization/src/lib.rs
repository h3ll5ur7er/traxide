//! Optimization Engine: makes routed copper shorter and simpler without
//! changing what it connects (docs/Architecture/Routing-pipeline.md#optimise).

use traxide_contracts::geometry::{Point, Segment};
use traxide_contracts::{ConnectionId, LayoutAccess, Routes, Track};

/// Measured on the corpus, a second pass saves some 3 % of the corners the
/// first one left, a third and fourth little more.
const MOST_PASSES: usize = 4;

/// `routes` with each track pulled tight: runs of its corners are replaced
/// by the shortest octilinear way between their ends wherever that fits in
/// `layout`, which holds the board but none of `routes`. A track's ends stay
/// where they are, and so do the vias, so everything stays joined as it was.
/// The tracks are taken in turn, each against the others as they are by then,
/// and again while that changes any: a track moved may make room for one
/// taken before it.
pub fn pull_tight(routes: &Routes, layout: &mut impl LayoutAccess) -> Routes {
    let vias = Routes {
        tracks: Vec::new(),
        vias: routes.vias.clone(),
    };
    layout.add_route(ConnectionId::from_index(0), &vias);
    let id_of = |track: usize| ConnectionId::from_index(track + 1);
    let alone = |track: &Track| Routes {
        tracks: vec![track.clone()],
        vias: Vec::new(),
    };
    let mut tracks = routes.tracks.clone();
    for (i, track) in tracks.iter().enumerate() {
        layout.add_route(id_of(i), &alone(track));
    }
    for _ in 0..MOST_PASSES {
        let mut changed = false;
        for (i, track) in tracks.iter_mut().enumerate() {
            let tight = tightened(track, layout);
            if tight != track.path {
                layout.remove_route(id_of(i));
                track.path = tight;
                layout.add_route(id_of(i), &alone(track));
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Routes {
        tracks,
        vias: vias.vias,
    }
}

/// The track's path with each run of corners that a shorter or simpler
/// octilinear way can replace replaced, the longest run first.
fn tightened(track: &Track, layout: &impl LayoutAccess) -> Vec<Point> {
    let path = &track.path;
    let fits = |way: &[Point]| {
        way.windows(2).all(|w| {
            layout.is_track_clear(
                track.net,
                track.layer,
                Segment { a: w[0], b: w[1] },
                track.width,
            )
        })
    };
    let mut tight = vec![path[0]];
    let mut i = 0;
    while i + 1 < path.len() {
        let after = |j: usize| path.get(j + 1).copied();
        let shortcut = (i + 2..path.len()).rev().find_map(|j| {
            ways_between(path[i], path[j])
                .into_iter()
                .filter(|way| improves(way, &path[i..=j]))
                .find(|way| turns_gently(&tight, way, after(j)) && fits(way))
                .map(|way| (j, way))
        });
        match shortcut {
            Some((j, way)) => {
                tight.extend(&way[1..]);
                i = j;
            }
            None => {
                tight.push(path[i + 1]);
                i += 1;
            }
        }
    }
    without_straight_corners(tight)
}

/// `path` without the corners that go on straight, where a way ended in the
/// direction the path goes on in.
fn without_straight_corners(path: Vec<Point>) -> Vec<Point> {
    let mut straight: Vec<Point> = Vec::with_capacity(path.len());
    for p in path {
        while let [.., a, b] = straight[..]
            && goes_on_straight(a, b, p)
        {
            straight.pop();
        }
        straight.push(p);
    }
    straight
}

fn goes_on_straight(a: Point, b: Point, c: Point) -> bool {
    let (u, v) = (b - a, c - b);
    let cross = i64::from(u.x) * i64::from(v.y) - i64::from(u.y) * i64::from(v.x);
    let dot = i64::from(u.x) * i64::from(v.x) + i64::from(u.y) * i64::from(v.y);
    cross == 0 && dot > 0
}

/// The shortest octilinear ways from `a` to `b`: straight where they lie on
/// one of the eight directions, else a diagonal and a straight stretch, in
/// either order.
fn ways_between(a: Point, b: Point) -> Vec<Vec<Point>> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    if a == b {
        return vec![vec![a]];
    }
    if dx == 0 || dy == 0 || dx.abs() == dy.abs() {
        return vec![vec![a, b]];
    }
    let side = dx.abs().min(dy.abs());
    let diagonal = Point::new(side * dx.signum(), side * dy.signum());
    vec![vec![a, a + diagonal, b], vec![a, b - diagonal, b]]
}

/// Whether `way` is shorter than `run`, or as long with fewer corners.
fn improves(way: &[Point], run: &[Point]) -> bool {
    let (shorter, as_long) = (length(way), length(run));
    shorter < as_long - 1.0 || (shorter <= as_long + 1.0 && way.len() < run.len())
}

fn length(path: &[Point]) -> f64 {
    path.windows(2)
        .map(|w| f64::from(w[1].x - w[0].x).hypot(f64::from(w[1].y - w[0].y)))
        .sum()
}

/// Whether the path `before`, then `way`, then on to `next`, turns by 90° at
/// most at each corner: a sharper corner traps etchant.
fn turns_gently(before: &[Point], way: &[Point], next: Option<Point>) -> bool {
    let from = before.len().checked_sub(2).map(|k| before[k]);
    let corners: Vec<Point> = from
        .into_iter()
        .chain(way.iter().copied())
        .chain(next)
        .collect();
    corners.windows(3).all(|w| {
        let (u, v) = (w[1] - w[0], w[2] - w[1]);
        i64::from(u.x) * i64::from(v.x) + i64::from(u.y) * i64::from(v.y) >= 0
    })
}
