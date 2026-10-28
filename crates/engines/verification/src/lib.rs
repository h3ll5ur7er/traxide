//! Verification Engine: how correctness is judged (volatility V9 of
//! docs/Architecture/Volatilities.md).
//!
//! It shares no clearance code with routing, only the geometry utility, so that a
//! bug in the router's checks cannot hide itself.

mod fill;
mod pieces;
mod unions;

use pieces::Piece;
use std::collections::BTreeSet;
use traxide_contracts::geometry::Point;
use traxide_contracts::{
    ConstraintSet, CopperItem, Design, KeepoutScope, LayerId, Limits, NetId, PadId, Routes,
    Verdict, Violation,
};
use unions::Unions;

/// Checks `routes`, which are all the copper on the board besides the pads,
/// against `rules` compiled for verification.
pub fn verify(design: &Design, rules: &ConstraintSet, routes: &Routes) -> Verdict {
    let pieces = pieces::collect(design, routes);
    // A set: one violation per pair of items, however many of their segments conflict.
    let mut found = BTreeSet::new();
    let mut joined = Unions::new(pieces.len());
    // The layers on which other copper of its net touches each via.
    let mut via_met: BTreeSet<(usize, LayerId)> = BTreeSet::new();
    for_each_pair_within(&pieces, rules.largest_clearance(), |i, j| {
        let (a, b) = (&pieces[i], &pieces[j]);
        if a.net.is_some() && a.net == b.net {
            if a.shape.is_closer_than(&b.shape, 1) {
                joined.join(i, j);
                for (via, other) in [(a, b), (b, a)] {
                    if let CopperItem::Via(index) = via.item
                        && other.item != via.item
                    {
                        via_met.insert((index, via.layer));
                    }
                }
            }
        } else if !(a.is_pad() && b.is_pad()) {
            // Pads too close to each other are the footprint's business, not
            // routing's; pads_too_close names them.
            let required = rules.clearance(a.net, b.net);
            if a.shape.is_closer_than(&b.shape, required) {
                let (first, second) = if a.item <= b.item {
                    (a.item, b.item)
                } else {
                    (b.item, a.item)
                };
                found.insert(Violation::Clearance {
                    layer: a.layer,
                    a: first,
                    b: second,
                    required,
                });
            }
        }
    });
    let mut violations: Vec<Violation> = found.into_iter().collect();
    violations.extend(holes_too_close(design, rules, routes));
    violations.extend(widths_out_of_limits(rules, routes));
    violations.extend(vias_out_of_limits(design, rules, routes));
    violations.extend(edge_and_keepout_violations(design, rules, &pieces));
    violations.extend(dangling_ends(design, routes, &pieces));
    violations.extend(dangling_vias(design, routes, &via_met));
    pieces::join_layers_of_the_same_item(&pieces, &mut joined);
    pieces::join_through_planes(design, rules, &pieces, &mut joined);
    violations.extend(unconnected(design, &pieces, &mut joined));
    Verdict { violations }
}

/// The tracks narrower or wider than the rules for their net allow, in order.
fn widths_out_of_limits(rules: &ConstraintSet, routes: &Routes) -> Vec<Violation> {
    routes
        .tracks
        .iter()
        .enumerate()
        .filter_map(|(index, track)| {
            let limits = rules.track_width_limits(track.net);
            let too_narrow = limits.min.is_some_and(|min| track.width < min);
            let too_wide = limits.max.is_some_and(|max| track.width > max);
            (too_narrow || too_wide).then_some(Violation::TrackWidth {
                item: CopperItem::Track(index),
                width: track.width,
                min: limits.min,
                max: limits.max,
            })
        })
        .collect()
}

/// The vias larger or smaller across, then those with a larger or smaller
/// drill, than the rules for their net allow, each in order. Across is the
/// width of a via's largest copper.
fn vias_out_of_limits(design: &Design, rules: &ConstraintSet, routes: &Routes) -> Vec<Violation> {
    let breaks = |value: i32, limits: Limits| {
        limits.min.is_some_and(|min| value < min) || limits.max.is_some_and(|max| value > max)
    };
    let mut diameters = Vec::new();
    let mut drills = Vec::new();
    for (index, via) in routes.vias.iter().enumerate() {
        let limits = rules.via_limits(via.net);
        let kind = design.via_kind(via.kind);
        let diameter = kind
            .copper
            .iter()
            .map(|c| {
                let bounds = c.shape.bounds();
                (bounds.max.x - bounds.min.x).max(bounds.max.y - bounds.min.y)
            })
            .max()
            .unwrap_or(0);
        if breaks(diameter, limits.diameter) {
            diameters.push(Violation::ViaDiameter {
                item: CopperItem::Via(index),
                diameter,
                min: limits.diameter.min,
                max: limits.diameter.max,
            });
        }
        if let Some(drill) = kind.drill.filter(|&drill| breaks(drill, limits.drill)) {
            drills.push(Violation::HoleSize {
                item: CopperItem::Via(index),
                drill,
                min: limits.drill.min,
                max: limits.drill.max,
            });
        }
    }
    diameters.extend(drills);
    diameters
}

/// The pairs of vias whose holes are closer than the hole-to-hole distance, in
/// index order. Vias of a net on the same spot are one hole, as KiCad drills it.
fn holes_too_close(design: &Design, rules: &ConstraintSet, routes: &Routes) -> Vec<Violation> {
    let required = rules.hole_to_hole();
    if required <= 0 {
        return Vec::new();
    }
    let holes: Vec<(usize, Point, i64)> = routes
        .vias
        .iter()
        .enumerate()
        .filter_map(|(i, via)| {
            let drill = design.via_kind(via.kind).drill?;
            Some((i, via.position, i64::from(drill / 2)))
        })
        .collect();
    let widest = holes
        .iter()
        .map(|&(_, _, radius)| radius)
        .max()
        .unwrap_or(0);
    let mut by_x: Vec<usize> = (0..holes.len()).collect();
    by_x.sort_by_key(|&h| holes[h].1.x);
    let mut found = BTreeSet::new();
    for (k, &h) in by_x.iter().enumerate() {
        let (i, at, radius) = holes[h];
        for &other in &by_x[k + 1..] {
            let (j, other_at, other_radius) = holes[other];
            if i64::from(other_at.x) - i64::from(at.x) >= 2 * widest + i64::from(required) {
                break;
            }
            let (via, other_via) = (&routes.vias[i], &routes.vias[j]);
            if via.net == other_via.net && at == other_at {
                continue;
            }
            if at.distance(other_at) - radius - other_radius < i64::from(required) {
                found.insert((i.min(j), i.max(j)));
            }
        }
    }
    found
        .into_iter()
        .map(|(a, b)| Violation::HoleToHole {
            a: CopperItem::Via(a),
            b: CopperItem::Via(b),
            required,
        })
        .collect()
}

/// The pairs of pads of different nets closer than their clearance, on any
/// layer, each in id order: connecting one of them breaks a rule, and where
/// they overlap, shorts the other. Pads of one part count only where they
/// overlap, as a footprint may set its pins closer by its own rules.
pub fn pads_too_close(design: &Design, rules: &ConstraintSet) -> Vec<[PadId; 2]> {
    let pads: Vec<Piece> = pieces::collect(design, &Routes::default());
    let mut found = BTreeSet::new();
    for_each_pair_within(&pads, rules.largest_clearance(), |i, j| {
        let (a, b) = (&pads[i], &pads[j]);
        let (CopperItem::Pad(first), CopperItem::Pad(second)) = (a.item, b.item) else {
            return;
        };
        let required = if design.pad(first).part() == design.pad(second).part() {
            1
        } else {
            rules.clearance(a.net, b.net)
        };
        if a.net.is_some()
            && b.net.is_some()
            && a.net != b.net
            && a.shape.is_closer_than(&b.shape, required)
        {
            found.insert([first.min(second), first.max(second)]);
        }
    });
    found.into_iter().collect()
}

/// Calls `visit` with each pair of pieces on one layer whose bounds lie
/// within `reach` of each other, in a sweep along x.
fn for_each_pair_within(pieces: &[Piece], reach: i32, mut visit: impl FnMut(usize, usize)) {
    let mut order: Vec<usize> = (0..pieces.len()).collect();
    order.sort_by_key(|&i| (pieces[i].layer, pieces[i].bounds.min.x, i));
    for (position, &i) in order.iter().enumerate() {
        let a = &pieces[i];
        let horizon = a.bounds.inflated(reach);
        for &j in order[position + 1..]
            .iter()
            .take_while(|&&j| pieces[j].layer == a.layer && pieces[j].bounds.min.x <= horizon.max.x)
        {
            if pieces[j].bounds.intersects(horizon) {
                visit(i, j);
            }
        }
    }
}

fn edge_and_keepout_violations(
    design: &Design,
    rules: &ConstraintSet,
    pieces: &[Piece],
) -> Vec<Violation> {
    let mut violations = BTreeSet::new();
    for piece in pieces.iter().filter(|p| !p.is_pad()) {
        if !design
            .outline
            .holds_apart(&piece.shape, rules.edge_clearance(), |hole| {
                rules.cutout_clearance(hole)
            })
        {
            violations.insert(Violation::Edge {
                layer: piece.layer,
                item: piece.item,
            });
        }
        let blocked_by = |scope: KeepoutScope| match (scope, piece.item) {
            (KeepoutScope::Everything, _) => true,
            (KeepoutScope::TracksOnly, item) => matches!(item, CopperItem::Track(_)),
            (KeepoutScope::ViasOnly, item) => matches!(item, CopperItem::Via(_)),
        };
        let hits_keepout = design.keepouts.iter().any(|k| {
            k.layer.is_none_or(|l| l == piece.layer)
                && blocked_by(k.scope)
                && piece.shape.is_closer_than(&k.shape, k.clearance)
        });
        if hits_keepout {
            violations.insert(Violation::Keepout {
                layer: piece.layer,
                item: piece.item,
            });
        }
    }
    violations.into_iter().collect()
}

/// The ends of tracks whose round end touches no other copper of their net on
/// their layer and lies in no pour of their net there, which KiCad's fill
/// would join to them; in the order of their layer, track and position.
fn dangling_ends(design: &Design, routes: &Routes, pieces: &[Piece]) -> Vec<Violation> {
    let widest = routes.tracks.iter().map(|t| t.width).max().unwrap_or(0);
    let mut ends: Vec<(LayerId, Point, usize)> = Vec::new();
    for (index, track) in routes.tracks.iter().enumerate() {
        if let [first, .., last] = track.path[..] {
            ends.push((track.layer, first, index));
            ends.push((track.layer, last, index));
        }
    }
    ends.sort_by_key(|&(layer, at, track)| (layer, at.x, at.y, track));
    let mut touched = vec![false; ends.len()];
    for piece in pieces {
        let Some(net) = piece.net else { continue };
        let reach = piece.bounds.inflated(widest / 2);
        let first =
            ends.partition_point(|&(layer, at, _)| (layer, at.x) < (piece.layer, reach.min.x));
        for (offset, &(layer, at, track)) in ends[first..].iter().enumerate() {
            if layer != piece.layer || at.x > reach.max.x {
                break;
            }
            let end = &routes.tracks[track];
            if end.net == net
                && piece.item != CopperItem::Track(track)
                && reach.contains(at)
                && piece.shape.nearest_point(at).distance(at) <= i64::from(end.width / 2)
            {
                touched[first + offset] = true;
            }
        }
    }
    let in_a_pour = |net: NetId, layer: LayerId, at: Point| {
        design.planes.iter().any(|plane| {
            plane.net == net
                && plane.layer == layer
                && plane.outline.contains(at)
                && !plane.holes.iter().any(|hole| hole.contains(at))
        })
    };
    let mut dangling: Vec<Violation> = ends
        .into_iter()
        .zip(touched)
        .filter(|&((layer, at, track), touched)| {
            !touched && !in_a_pour(routes.tracks[track].net, layer, at)
        })
        .map(|((layer, at, track), _)| Violation::Dangling {
            layer,
            item: CopperItem::Track(track),
            at,
        })
        .collect();
    dangling.sort();
    dangling
}

/// The vias that copper of their net, or a pour or plane of it, joins on fewer
/// than two of their layers: a via that leads nowhere. Each is named on a
/// layer it is joined on, else on its first.
fn dangling_vias(
    design: &Design,
    routes: &Routes,
    met: &BTreeSet<(usize, LayerId)>,
) -> Vec<Violation> {
    let in_a_pour = |net: NetId, layer: LayerId, at: Point| {
        design.planes.iter().any(|plane| {
            plane.net == net
                && plane.layer == layer
                && plane.outline.contains(at)
                && !plane.holes.iter().any(|hole| hole.contains(at))
        })
    };
    let mut dangling = Vec::new();
    for (index, via) in routes.vias.iter().enumerate() {
        let layers: Vec<LayerId> = design
            .via_kind(via.kind)
            .copper
            .iter()
            .map(|c| c.layer)
            .collect();
        let joined: Vec<LayerId> = layers
            .iter()
            .copied()
            .filter(|&layer| {
                met.contains(&(index, layer)) || in_a_pour(via.net, layer, via.position)
            })
            .collect();
        if joined.len() < 2 {
            dangling.push(Violation::Dangling {
                layer: joined
                    .first()
                    .or(layers.first())
                    .copied()
                    .unwrap_or(LayerId(0)),
                item: CopperItem::Via(index),
                at: via.position,
            });
        }
    }
    dangling
}

fn unconnected(design: &Design, pieces: &[Piece], joined: &mut Unions) -> Vec<Violation> {
    let piece_of_pad = |pad| pieces.iter().position(|p| p.item == CopperItem::Pad(pad));
    let mut violations = Vec::new();
    for (index, net) in design.nets.iter().enumerate() {
        let Some((&first, rest)) = net.pads.split_first() else {
            continue;
        };
        let Some(anchor) = piece_of_pad(first) else {
            continue;
        };
        let apart: Vec<_> = rest
            .iter()
            .copied()
            .filter(|&pad| piece_of_pad(pad).is_none_or(|p| !joined.same(anchor, p)))
            .collect();
        if !apart.is_empty() {
            violations.push(Violation::Unconnected {
                net: NetId::from_index(index),
                pads: apart,
            });
        }
    }
    violations
}
