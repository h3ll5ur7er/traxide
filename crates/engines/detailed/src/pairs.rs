//! Differential pairs routed coupled (TRX-38): the pair's two tracks are
//! searched as one centre line as wide as both and the gap between them, and
//! split at the gap into the two, so that they run side by side all the way.
//!
//! At each end the centre line starts at a launch point in front of the
//! pair's two pads, where its copper keeps clear of theirs; each pad's track
//! runs straight out to the edge of its pad, at 45° to where the tracks run a
//! pitch apart, and straight on to the launch point. Where that does not fit,
//! as among the pins of a chip's fan-out, the tracks may stop short of the
//! pads at one end, at the launch point, for the router to join them there as
//! it joins any lane of wiring a pad starts.
//!
//! Where the centre line changes layers, the pair's two vias stand side by
//! side across it, farther apart than its tracks, and farther still where it
//! turns there; the search checks a disc that holds them however they stand.

use crate::path;
use crate::search::{Scratch, Search};
use traxide_contracts::geometry::{Point, Segment, Shape};
use traxide_contracts::{
    Connection, ConnectionId, ConstraintSet, CoupledConnections, Design, DifferentialPair, LayerId,
    LayerShape, LayoutAccess, NetId, Pad, PadId, Profile, Routes, Track, Via, ViaKindId,
};

/// How far in front of its pads, in pair pitches, a pair's launch point may
/// lie: its centre line clears pads a few pitches across, not larger ones.
const FARTHEST_LAUNCH_IN_PAIR_PITCHES: i32 = 4;

/// Routes each of `coupled` as two tracks side by side at its pair's gap,
/// where they fit in `layout`, and adds them to it: what one pair routes, the
/// next keeps clear of. The pairs that do not fit, `None`, are left to route
/// alone.
pub fn route_pairs(
    design: &Design,
    rules: &ConstraintSet,
    profile: &Profile,
    coupled: &[CoupledConnections],
    layout: &mut impl LayoutAccess,
) -> Vec<Option<Routes>> {
    let mut scratch = Scratch::default();
    (coupled.iter().enumerate())
        .map(|(index, pair)| {
            let mut routes = couple(design, rules, profile, pair, layout, &mut scratch)?;
            let id = ConnectionId::from_index(index);
            layout.add_route(id, &routes);
            let stitching = Routes {
                tracks: Vec::new(),
                vias: stitching_vias(design, rules, layout, &routes, pair.pair),
            };
            layout.add_route(id, &stitching);
            routes.extend(stitching);
            Some(routes)
        })
        .collect()
}

/// Beside each pair of vias where `routes`, `pair`'s, change layers, a via
/// of the net whose planes and pours cover most of their layers there, two
/// at least: the current returning under the pair follows it from layer to
/// layer. Outside the positive net's via, else outside the negative's, else
/// ahead or behind the pair, wherever it first fits in `layout`.
fn stitching_vias(
    design: &Design,
    rules: &ConstraintSet,
    layout: &impl LayoutAccess,
    routes: &Routes,
    pair: DifferentialPair,
) -> Vec<Via> {
    let of = |net: NetId| routes.vias.iter().filter(move |v| v.net == net);
    let mut stitching = Vec::new();
    for (positive, negative) in of(pair.positive).zip(of(pair.negative)) {
        let kind = design.via_kind(positive.kind);
        let poured = |net: NetId, at: Point| {
            (kind.copper.iter())
                .filter(|c| {
                    design.planes.iter().any(|plane| {
                        plane.net == net
                            && plane.layer == c.layer
                            && plane.outline.contains(at)
                            && !plane.holes.iter().any(|hole| hole.contains(at))
                    })
                })
                .count()
        };
        let middle = midpoint(positive.position, negative.position);
        let reference = (design.planes.iter().map(|plane| plane.net))
            .filter(|&net| net != pair.positive && net != pair.negative)
            .map(|net| (poured(net, middle), std::cmp::Reverse(net)))
            .filter(|&(layers, _)| layers >= 2)
            .max()
            .map(|(_, std::cmp::Reverse(net))| net);
        let Some(reference) = reference else {
            continue;
        };
        let diameter = (kind.copper.iter())
            .map(|c| {
                let bounds = c.shape.bounds();
                (bounds.max.x - bounds.min.x).max(bounds.max.y - bounds.min.y)
            })
            .max()
            .unwrap_or(0);
        let apart = f64::from(diameter + rules.clearance(Some(reference), Some(pair.positive)) + 1);
        let across = unit(negative.position, positive.position);
        let along = left(across);
        let half = positive.position.distance(negative.position) as f64 / 2.0;
        let beside = (apart * apart - half * half).max(0.0).sqrt();
        let candidates = [
            moved(positive.position, across, apart),
            moved(negative.position, across, -apart),
            moved(middle, along, beside),
            moved(middle, along, -beside),
        ];
        let found = candidates.into_iter().find_map(|at| {
            let via = Via {
                net: reference,
                kind: positive.kind,
                position: at,
            };
            (poured(reference, at) >= 2 && layout.is_via_clear(&via)).then_some(via)
        });
        stitching.extend(found);
    }
    stitching
}

/// A pair at one end: the positive net's pad and the negative's, and half
/// the distance the tracks' centres run apart.
#[derive(Clone, Copy)]
struct End<'d> {
    positive: &'d Pad,
    negative: &'d Pad,
    half_pitch: i32,
}

/// Where a pair's tracks leave its pads at one end: each pad's track runs
/// straight out to the edge of its pad, `edges`, and at 45° in or out to
/// where the tracks run a pitch apart, either side of `base`; from there
/// straight on to `at`, where the centre line starts.
#[derive(Debug, Clone, Copy)]
struct Launch {
    layer: LayerId,
    edges: [Point; 2],
    base: Point,
    at: Point,
}

impl End<'_> {
    /// The launches in front of the pads one way or the other on `layer`,
    /// each at the nearest point on the grid of `pitch` from which a line
    /// `wide` across keeps clear of the copper around.
    fn launches(
        self,
        layout: &impl LayoutAccess,
        net: NetId,
        layer: LayerId,
        wide: i32,
        pitch: i32,
    ) -> Vec<Launch> {
        let (p, n) = (self.positive.position, self.negative.position);
        let across = (f64::from(n.x - p.x), f64::from(n.y - p.y));
        let apart = across.0.hypot(across.1);
        if apart == 0.0 {
            return Vec::new();
        }
        let middle = midpoint(p, n);
        let fanning = (apart / 2.0 - f64::from(self.half_pitch)).abs();
        let farthest = FARTHEST_LAUNCH_IN_PAIR_PITCHES * 2 * self.half_pitch;
        [1.0, -1.0]
            .into_iter()
            .filter_map(|sign| {
                let ahead = unit_of((-across.1 * sign, across.0 * sign));
                let edge = [self.positive, self.negative]
                    .iter()
                    .filter_map(|pad| reach_along(pad, layer, ahead))
                    .fold(0.0, f64::max);
                let edges = [p, n].map(|pad| moved(pad, ahead, edge));
                let base = moved(middle, ahead, edge + fanning);
                (1..=farthest / pitch).find_map(|steps| {
                    let at = on_grid(moved(base, ahead, f64::from(steps * pitch)), pitch);
                    let is_ahead = f64::from(at.x - base.x) * ahead.0
                        + f64::from(at.y - base.y) * ahead.1
                        > 0.0;
                    (is_ahead && layout.is_track_clear(net, layer, Segment::new(at, at), wide))
                        .then_some(Launch {
                            layer,
                            edges,
                            base,
                            at,
                        })
                })
            })
            .collect()
    }
}

/// How far `pad`'s copper on `layer` reaches from its centre along
/// `direction`.
fn reach_along(pad: &Pad, layer: LayerId, direction: (f64, f64)) -> Option<f64> {
    let bounds = pad.copper.iter().find(|c| c.layer == layer)?.shape.bounds();
    let corners = [
        (bounds.min.x, bounds.min.y),
        (bounds.min.x, bounds.max.y),
        (bounds.max.x, bounds.min.y),
        (bounds.max.x, bounds.max.y),
    ];
    corners
        .iter()
        .map(|&(x, y)| {
            f64::from(x - pad.position.x) * direction.0
                + f64::from(y - pad.position.y) * direction.1
        })
        .reduce(f64::max)
}

/// The two tracks of `coupled`, and their vias, from the launch points where
/// they are shortest.
fn couple(
    design: &Design,
    rules: &ConstraintSet,
    profile: &Profile,
    coupled: &CoupledConnections,
    layout: &impl LayoutAccess,
    scratch: &mut Scratch,
) -> Option<Routes> {
    let pair = coupled.pair;
    let [positive, negative] = coupled.connections;
    let gap = pair
        .gap
        .max(rules.clearance(Some(pair.positive), Some(pair.negative)));
    // A unit more, as the split tracks' corners round to the unit.
    let half_pitch = (pair.width + gap) / 2 + 1;
    let widest_via = (rules.via_kinds(pair.positive).iter())
        .flat_map(|&kind| &design.via_kind(kind).copper)
        .map(|copper| {
            let bounds = copper.shape.bounds();
            (bounds.max.x - bounds.min.x).max(bounds.max.y - bounds.min.y)
        })
        .max()
        .unwrap_or(0);
    let geometry = Geometry {
        half_pitch,
        via_half_pitch: (widest_via + gap) / 2 + 1,
        via_to_track: widest_via / 2 + pair.width / 2 + gap + 1,
        width: pair.width,
    };
    let wide = 2 * half_pitch + pair.width;
    let ends = [
        End {
            positive: design.pad(positive.from),
            negative: design.pad(negative.from),
            half_pitch,
        },
        End {
            positive: design.pad(positive.to),
            negative: design.pad(negative.to),
            half_pitch,
        },
    ];
    let pitch = profile.grid_pitch;
    let layers_of = |end: End| -> Vec<LayerId> {
        design
            .layer_ids()
            .filter(|&layer| design.layers[layer.index()].routable)
            .filter(|&layer| end.positive.is_on(layer) && end.negative.is_on(layer))
            .collect()
    };
    let launches_of = |end: End| -> Vec<Launch> {
        layers_of(end)
            .into_iter()
            .flat_map(|layer| end.launches(layout, pair.positive, layer, wide, pitch))
            .collect()
    };
    let (starts, finishes) = (launches_of(ends[0]), launches_of(ends[1]));
    let via_disc = 2 * geometry.via_half_pitch_turning(NINETY_DEGREES) + widest_via;
    let mut best: Option<(i64, Routes)> = None;
    for &first in &starts {
        for &last in &finishes {
            let launches = [(first.at, first.layer), (last.at, last.layer)];
            let Some(searched) = centre_line(
                design,
                rules,
                profile,
                pair.positive,
                launches,
                [wide, via_disc],
                layout,
                scratch,
            ) else {
                continue;
            };
            for reaching in [[true, true], [true, false], [false, true]] {
                let Some((cost, routes)) = split_from(
                    &searched,
                    reaching,
                    [first, last],
                    (pair, geometry, ends),
                    profile.via_cost,
                ) else {
                    continue;
                };
                if best.as_ref().is_some_and(|(cheapest, _)| *cheapest <= cost)
                    || !fits(design, rules, layout, &routes, pair)
                {
                    continue;
                }
                best = Some((cost, routes));
            }
        }
    }
    best.map(|(_, routes)| routes)
}

/// The pair's tracks along `searched`, reaching their pads at the ends that
/// `reaching` says, through the launches' fan-ins, and what they cost: their
/// length, their vias, and at an end left short of its pads, a via and the
/// distance to each pad, for the router to join them there.
fn split_from(
    searched: &[Stretch],
    reaching: [bool; 2],
    launches: [Launch; 2],
    (pair, geometry, ends): (DifferentialPair, Geometry, [End; 2]),
    via_cost: i64,
) -> Option<(i64, Routes)> {
    let mut stretches = searched.to_vec();
    if reaching[0] {
        stretches[0].path.insert(0, launches[0].base);
    }
    if reaching[1] {
        stretches.last_mut()?.path.push(launches[1].base);
    }
    for stretch in &mut stretches {
        stretch.path.dedup();
        if turns_sharply(&stretch.path) {
            return None;
        }
    }
    let side = side_of(&stretches, reaching, ends)?;
    let routes = split(&stretches, pair, side, geometry, ends, launches, reaching)?;
    let length = routes.tracks.iter().map(track_length).sum::<i64>();
    let vias = routes.vias.len() as i64;
    let free_end = |end: usize, net: NetId| match end {
        0 => (routes.tracks.iter().find(|t| t.net == net)).map(|t| t.path[0]),
        _ => (routes.tracks.iter().rfind(|t| t.net == net)).and_then(|t| t.path.last().copied()),
    };
    let left_short: i64 = (0..2)
        .filter(|&end| !reaching[end])
        .map(|end| {
            let pads = [
                (ends[end].positive, pair.positive),
                (ends[end].negative, pair.negative),
            ];
            let to_join = pads.iter().map(|&(pad, net)| {
                free_end(end, net).map_or(0, |free| pad.position.distance(free))
            });
            via_cost + to_join.sum::<i64>()
        })
        .sum();
    Some((length + vias * via_cost + left_short, routes))
}

/// How far apart a pair's tracks run, and its vias stand where it runs
/// straight on, halved; and how far a via's centre keeps from the other
/// net's track's.
#[derive(Clone, Copy)]
struct Geometry {
    half_pitch: i32,
    via_half_pitch: i32,
    via_to_track: i32,
    width: i32,
}

/// The cosine of half a turn of 90°, the sharpest a pair takes at its vias:
/// a sharper one would cross its tracks.
const NINETY_DEGREES: f64 = std::f64::consts::FRAC_1_SQRT_2;

impl Geometry {
    /// How far from the centre line its vias stand where it turns by an
    /// angle whose half has `half_cosine`: they stand across the bisector of
    /// the turn, which brings each nearer the other net's track as the turn
    /// grows, so they stand farther apart.
    fn via_half_pitch_turning(self, half_cosine: f64) -> i32 {
        let clear_of_tracks = f64::from(self.via_to_track - self.half_pitch) / half_cosine;
        self.via_half_pitch.max(clear_of_tracks.ceil() as i32)
    }
}

/// A stretch of the centre line on one layer, and the kind of via that
/// joins it to the next.
#[derive(Clone)]
struct Stretch {
    layer: LayerId,
    path: Vec<Point>,
    onwards: Option<ViaKindId>,
}

/// The centre line of a pair `widths[0]` across between `launches`, on their
/// layers, searched as the positive net's; its vias for the pair fit in a
/// disc `widths[1]` across.
#[allow(clippy::too_many_arguments)]
fn centre_line(
    design: &Design,
    rules: &ConstraintSet,
    profile: &Profile,
    net: NetId,
    launches: [(Point, LayerId); 2],
    [wide, via_disc]: [i32; 2],
    layout: &impl LayoutAccess,
    scratch: &mut Scratch,
) -> Option<Vec<Stretch>> {
    let mut with_launches = design.clone();
    let [from, to] = launches.map(|(at, layer)| {
        let pad = PadId::from_index(with_launches.pads.len());
        with_launches.pads.push(Pad {
            name: "~launch".into(),
            net: Some(net),
            position: at,
            // A point on the grid: the search leaves it by a step of the grid.
            copper: vec![LayerShape {
                layer,
                shape: Shape::circle(at, 2),
            }],
        });
        pad
    });
    let connection = Connection { net, from, to };
    let search = Search::new(
        &with_launches,
        rules,
        profile,
        connection,
        layout,
        None,
        None,
    )
    .with_width(wide)
    .with_pair_vias(via_disc);
    let steps = search.run(scratch).ok()?;
    let routes = path::to_routes(&steps, net, wide);
    let joined_by_vias = routes.tracks.len() == routes.vias.len() + 1
        && (routes.tracks.windows(2).zip(&routes.vias)).all(|(pair, via)| {
            pair[0].path.last() == Some(&via.position)
                && pair[1].path.first() == Some(&via.position)
        });
    let onwards = (routes.vias.iter().map(|via| Some(via.kind))).chain([None]);
    joined_by_vias.then(|| {
        (routes.tracks.into_iter().zip(onwards))
            .map(|(track, onwards)| Stretch {
                layer: track.layer,
                path: track.path,
                onwards,
            })
            .collect()
    })
}

/// The pair's two tracks along `stretches`, the positive net's on `side` of
/// it, each from its pad at one of `ends` to its pad at the other, at the
/// ends `reaching` says; at the other, from where the stretches start. Where the
/// centre line changes layers, the pair's vias stand side by side across it,
/// on the line halfway between the stretches' sides; a centre line that
/// turns back there by more than 90° would cross the tracks.
fn split(
    stretches: &[Stretch],
    pair: DifferentialPair,
    side: i32,
    geometry: Geometry,
    ends: [End; 2],
    [first, last]: [Launch; 2],
    reaching: [bool; 2],
) -> Option<Routes> {
    let mut routes = Routes::default();
    let nets = [
        (pair.positive, side, [ends[0].positive, ends[1].positive], 0),
        (
            pair.negative,
            -side,
            [ends[0].negative, ends[1].negative],
            1,
        ),
    ];
    for (net, sign, [from, to], which) in nets {
        let mut path = Vec::new();
        if reaching[0] {
            extend_path(&mut path, &[from.position, first.edges[which]]);
        }
        for (i, stretch) in stretches.iter().enumerate() {
            extend_path(
                &mut path,
                &offset_line(&stretch.path, sign * geometry.half_pitch),
            );
            let Some(next) = stretches.get(i + 1) else {
                if reaching[1] {
                    extend_path(&mut path, &[last.edges[which], to.position]);
                }
                routes.tracks.push(Track {
                    net,
                    layer: stretch.layer,
                    width: geometry.width,
                    path: std::mem::take(&mut path),
                });
                break;
            };
            let n = stretch.path.len();
            let before = left(unit(stretch.path[n - 2], stretch.path[n - 1]));
            let after = left(unit(next.path[0], next.path[1]));
            let cosine = before.0 * after.0 + before.1 * after.1;
            if cosine < 0.0 {
                return None;
            }
            let half_cosine = f64::midpoint(1.0, cosine).sqrt();
            let across = unit_of((before.0 + after.0, before.1 + after.1));
            let apart = sign * geometry.via_half_pitch_turning(half_cosine);
            let at = moved(next.path[0], across, f64::from(apart));
            extend_path(&mut path, &[at]);
            routes.tracks.push(Track {
                net,
                layer: stretch.layer,
                width: geometry.width,
                path: std::mem::replace(&mut path, vec![at]),
            });
            routes.vias.push(Via {
                net,
                kind: stretch.onwards?,
                position: at,
            });
        }
    }
    Some(routes)
}

/// Whether `path` turns by more than 90° anywhere, as the search never
/// does, but a centre line may where it leaves a launch point backwards.
fn turns_sharply(path: &[Point]) -> bool {
    path.windows(3).any(|w| {
        let (before, after) = (unit(w[0], w[1]), unit(w[1], w[2]));
        before.0 * after.0 + before.1 * after.1 < -1e-9
    })
}

/// `path` on along `points`, each once.
fn extend_path(path: &mut Vec<Point>, points: &[Point]) {
    for &at in points {
        if path.last() != Some(&at) {
            path.push(at);
        }
    }
}

/// Which side of the centre line the positive net's pads lie on, +1 to the
/// left of its way and -1 to the right, at the ends whose pads it reaches;
/// none where they lie on different sides at its two ends, as the tracks
/// would have to cross, or on it, or where it reaches neither end's pads.
fn side_of(stretches: &[Stretch], reaching: [bool; 2], ends: [End; 2]) -> Option<i32> {
    let side = |from: Point, to: Point, pad: Point| {
        let (dx, dy) = (i64::from(to.x - from.x), i64::from(to.y - from.y));
        let (px, py) = (i64::from(pad.x - from.x), i64::from(pad.y - from.y));
        (dx * py - dy * px).signum() as i32
    };
    let (start, end) = (&stretches.first()?.path, &stretches.last()?.path);
    if start.len() < 2 || end.len() < 2 {
        return None;
    }
    let at_start = side(start[0], start[1], ends[0].positive.position);
    let at_end = side(
        end[end.len() - 2],
        end[end.len() - 1],
        ends[1].positive.position,
    );
    match reaching {
        [true, true] => (at_start != 0 && at_start == at_end).then_some(at_start),
        [true, false] => (at_start != 0).then_some(at_start),
        [false, true] => (at_end != 0).then_some(at_end),
        [false, false] => None,
    }
}

/// `centre` moved `offset` to the left of its way, or to the right where
/// negative. On the inside of a bend the moved stretches meet; on the outside
/// the corner is cut, so that it comes no farther from the centre line's
/// corner than `offset`, and stays inside the copper of the centre line.
fn offset_line(centre: &[Point], offset: i32) -> Vec<Point> {
    let offset = f64::from(offset);
    let directions: Vec<(f64, f64)> = centre.windows(2).map(|w| unit(w[0], w[1])).collect();
    let mut line = vec![moved(centre[0], left(directions[0]), offset)];
    for (i, pair) in directions.windows(2).enumerate() {
        let (before, after) = (pair[0], pair[1]);
        let (into, out_of) = (left(before), left(after));
        let corner = centre[i + 1];
        let turn = before.0 * after.1 - before.1 * after.0;
        if turn.abs() < 1e-9 {
            line.push(moved(corner, into, offset));
        } else if (turn > 0.0) == (offset > 0.0) {
            let along = (into.0 + out_of.0, into.1 + out_of.1);
            let cosine = before.0 * after.0 + before.1 * after.1;
            line.push(moved(corner, along, offset / (1.0 + cosine)));
        } else {
            line.push(moved(corner, into, offset));
            line.push(moved(corner, out_of, offset));
        }
    }
    let last = *centre.last().expect("a centre line of two points at least");
    line.push(moved(
        last,
        left(*directions.last().expect("a step")),
        offset,
    ));
    line
}

/// The direction from `a` to `b`, of length one.
fn unit(a: Point, b: Point) -> (f64, f64) {
    unit_of((f64::from(b.x - a.x), f64::from(b.y - a.y)))
}

fn unit_of((x, y): (f64, f64)) -> (f64, f64) {
    let length = x.hypot(y);
    (x / length, y / length)
}

/// `direction` turned a quarter to the left.
fn left((x, y): (f64, f64)) -> (f64, f64) {
    (-y, x)
}

fn moved(at: Point, (x, y): (f64, f64), by: f64) -> Point {
    Point::new(
        (f64::from(at.x) + x * by).round() as i32,
        (f64::from(at.y) + y * by).round() as i32,
    )
}

/// Whether `routes`, `pair`'s, fit in `layout` and its two nets' copper
/// keeps clear of each other.
fn fits(
    design: &Design,
    rules: &ConstraintSet,
    layout: &impl LayoutAccess,
    routes: &Routes,
    pair: DifferentialPair,
) -> bool {
    let segments = |track: &Track| {
        (track.path.windows(2).map(|w| Segment::new(w[0], w[1]))).collect::<Vec<_>>()
    };
    let in_layout = routes.tracks.iter().all(|track| {
        (segments(track).into_iter())
            .all(|segment| layout.is_track_clear(track.net, track.layer, segment, track.width))
    }) && routes.vias.iter().all(|via| layout.is_via_clear(via));
    if !in_layout {
        return false;
    }
    let copper = |net: NetId| -> Vec<(LayerId, Shape)> {
        let tracks = (routes.tracks.iter().filter(|t| t.net == net)).flat_map(|track| {
            (segments(track).into_iter())
                .map(|segment| (track.layer, Shape::track(segment, track.width)))
        });
        let vias = (routes.vias.iter().filter(|v| v.net == net)).flat_map(|via| {
            (design.via_kind(via.kind).copper.iter()).map(|c| {
                let at = via.position;
                (c.layer, c.shape.map(|p| Point::new(p.x + at.x, p.y + at.y)))
            })
        });
        tracks.chain(vias).collect()
    };
    let (ours, theirs) = (copper(pair.positive), copper(pair.negative));
    let clearance = rules.clearance(Some(pair.positive), Some(pair.negative));
    ours.iter().all(|(layer, shape)| {
        (theirs.iter().filter(|(l, _)| l == layer))
            .all(|(_, other)| !shape.is_closer_than(other, clearance))
    })
}

fn on_grid(at: Point, pitch: i32) -> Point {
    let snap = |v: i32| (v + pitch / 2).div_euclid(pitch) * pitch;
    Point::new(snap(at.x), snap(at.y))
}

fn midpoint(a: Point, b: Point) -> Point {
    Point::new(
        ((i64::from(a.x) + i64::from(b.x)) / 2) as i32,
        ((i64::from(a.y) + i64::from(b.y)) / 2) as i32,
    )
}

fn track_length(track: &Track) -> i64 {
    track.path.windows(2).map(|w| w[0].distance(w[1])).sum()
}
