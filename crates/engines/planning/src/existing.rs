use std::collections::{HashMap, HashSet};
use traxide_contracts::geometry::{Point, Rect, Segment, Shape};
use traxide_contracts::{Design, LayerId, NetId, Plane};

/// For each pad, the smallest index of the pads that the board's own copper of
/// its net, its planes included, joins it to: pads with the same number need no
/// connection between them. Boards often come partly routed.
pub(crate) fn joined_pads(design: &Design) -> Vec<usize> {
    joins(design).representative
}

/// For each pad, whether the board's own copper of its net joins it to one of
/// the net's planes.
pub(crate) fn pads_on_planes(design: &Design) -> Vec<bool> {
    joins(design).on_a_plane
}

/// Whether copper of a plane's net shaped `shape` on the plane's layer joins
/// it: where it lies on the plane, and no plane of another net that it gives
/// way to overlaps it. Only on a layer that takes no tracks: elsewhere routes
/// of other nets may cut a pour apart.
pub(crate) fn joins_plane(design: &Design, plane: &Plane, shape: &Shape) -> bool {
    !design.layers[plane.layer.index()].routable
        && shape.is_inside(&plane.outline, 0)
        && !(plane.holes.iter()).any(|hole| Shape::Polygon(hole.clone()).is_closer_than(shape, 0))
        && !design.planes.iter().any(|other| {
            plane.gives_way_to(other)
                && Shape::Polygon(other.outline.clone()).is_closer_than(shape, 0)
        })
}

struct Joins {
    representative: Vec<usize>,
    on_a_plane: Vec<bool>,
}

fn joins(design: &Design) -> Joins {
    let mut pieces: Vec<Piece> = Vec::new();
    for (index, pad) in design.pads.iter().enumerate() {
        let Some(net) = pad.net else { continue };
        for copper in &pad.copper {
            pieces.push(Piece::new(
                Owner::Pad(index),
                net,
                copper.layer,
                copper.shape.clone(),
            ));
        }
    }
    let first_track = pieces.len();
    for (index, track) in design.existing.tracks.iter().enumerate() {
        for pair in track.path.windows(2) {
            let shape = Shape::track(Segment::new(pair[0], pair[1]), track.width);
            pieces.push(Piece::new(
                Owner::Track(index),
                track.net,
                track.layer,
                shape,
            ));
        }
    }
    for (index, via) in design.existing.vias.iter().enumerate() {
        let offset = via.position;
        for copper in &design.via_kind(via.kind).copper {
            let shape = copper
                .shape
                .map(|p| Point::new(p.x + offset.x, p.y + offset.y));
            pieces.push(Piece::new(Owner::Via(index), via.net, copper.layer, shape));
        }
    }
    let mut groups = Groups::new(pieces.len());
    // Pieces of one owner, such as the layers of a via, are joined already.
    for (a, b) in (1..pieces.len()).map(|i| (i - 1, i)) {
        if pieces[a].owner == pieces[b].owner {
            groups.join(a, b);
        }
    }
    // A sweep along x: only pieces whose spans overlap can touch. Only the
    // board's own copper joins pads, so each pair needs one piece of it.
    let mut by_left_edge: Vec<usize> = (0..pieces.len()).collect();
    by_left_edge.sort_by_key(|&i| pieces[i].bounds.min.x);
    for (k, &a) in by_left_edge.iter().enumerate() {
        for &b in &by_left_edge[k + 1..] {
            if pieces[b].bounds.min.x > pieces[a].bounds.max.x {
                break;
            }
            let is_existing = |i: usize| i >= first_track;
            if (is_existing(a) || is_existing(b)) && pieces[a].touches(&pieces[b]) {
                groups.join(a, b);
            }
        }
    }
    let mut on_planes = Vec::new();
    for plane in &design.planes {
        let on: Vec<usize> = (0..pieces.len())
            .filter(|&i| {
                let piece = &pieces[i];
                piece.net == plane.net
                    && piece.layer == plane.layer
                    && joins_plane(design, plane, &piece.shape)
            })
            .collect();
        for pair in on.windows(2) {
            groups.join(pair[0], pair[1]);
        }
        on_planes.extend(on);
    }
    let plane_groups: HashSet<usize> = on_planes.into_iter().map(|i| groups.root(i)).collect();
    let mut representative = vec![usize::MAX; design.pads.len()];
    let mut on_a_plane = vec![false; design.pads.len()];
    let mut first_pad_of_group = HashMap::new();
    for (index, piece) in pieces.iter().enumerate() {
        if let Owner::Pad(pad) = piece.owner {
            let group = groups.root(index);
            let first = *first_pad_of_group.entry(group).or_insert(pad);
            representative[pad] = first;
            on_a_plane[pad] = plane_groups.contains(&group);
        }
    }
    Joins {
        representative: representative
            .into_iter()
            .enumerate()
            .map(|(pad, first)| if first == usize::MAX { pad } else { first })
            .collect(),
        on_a_plane,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    Pad(usize),
    Track(usize),
    Via(usize),
}

struct Piece {
    owner: Owner,
    net: NetId,
    layer: LayerId,
    shape: Shape,
    bounds: Rect,
}

impl Piece {
    fn new(owner: Owner, net: NetId, layer: LayerId, shape: Shape) -> Self {
        Self {
            owner,
            net,
            layer,
            bounds: shape.bounds(),
            shape,
        }
    }

    fn touches(&self, other: &Self) -> bool {
        self.net == other.net
            && self.layer == other.layer
            && self.bounds.intersects(other.bounds)
            && self.shape.is_closer_than(&other.shape, 1)
    }
}

/// Union-find over pieces of copper.
struct Groups {
    parent: Vec<usize>,
}

impl Groups {
    fn new(count: usize) -> Self {
        Self {
            parent: (0..count).collect(),
        }
    }

    fn root(&mut self, mut item: usize) -> usize {
        while self.parent[item] != item {
            self.parent[item] = self.parent[self.parent[item]];
            item = self.parent[item];
        }
        item
    }

    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.root(a), self.root(b));
        self.parent[a.max(b)] = a.min(b);
    }
}
