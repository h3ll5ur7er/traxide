//! Layout Access: how the routing state is stored and queried (volatility V11 of
//! docs/Architecture/Volatilities.md). For now a uniform grid of buckets per
//! layer, queried one item at a time.

mod buckets;
mod outline;

use buckets::Buckets;
use outline::Outline;
use traxide_contracts::geometry::{Point, Rect, Segment, Shape};
use traxide_contracts::{
    ConnectionId, ConstraintSet, Design, KeepoutScope, LayerId, LayoutAccess, NetId, Obstruction,
    Routes, Via, ViaKindId,
};

/// Buckets of about this size keep a clearance query to a handful of buckets for
/// the track widths and clearances of ordinary boards.
const BUCKET_SIZE: i32 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    /// Copper that stays: pads and locked tracks.
    Fixed(Option<NetId>),
    /// Copper routed for a connection, which rip-up may take away again.
    Routed {
        net: NetId,
        connection: ConnectionId,
    },
    /// Not copper: copper keeps the keepout's own clearance from it.
    Keepout(KeepoutScope, i32),
}

#[derive(Debug, Clone)]
struct Obstacle {
    owner: Owner,
    layer: LayerId,
    shape: Shape,
    bounds: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Track,
    Via,
}

/// Whether a query needs every obstruction or only whether there is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wanted {
    All,
    First,
}

/// The copper and keepouts of a design, and the routes added so far.
#[derive(Debug)]
pub struct Layout<'a> {
    design: &'a Design,
    rules: &'a ConstraintSet,
    outline: Outline,
    obstacles: Vec<Obstacle>,
    layers: Vec<Buckets>,
    /// The obstacles of each routed connection, by connection.
    routed: Vec<Vec<usize>>,
    /// The holes of vias, which keep the hole-to-hole distance whatever their
    /// nets, indexed by `hole_buckets`.
    holes: Vec<Hole>,
    hole_buckets: Buckets,
    widest_hole: i32,
    /// The holes of each routed connection, by connection.
    routed_holes: Vec<Vec<usize>>,
}

#[derive(Debug, Clone)]
struct Hole {
    net: NetId,
    kind: ViaKindId,
    position: Point,
    radius: i32,
    connection: Option<ConnectionId>,
}

impl Hole {
    fn bounds(&self) -> Rect {
        Rect {
            min: self.position,
            max: self.position,
        }
        .inflated(self.radius)
    }
}

impl<'a> Layout<'a> {
    /// Everything routing must keep clear of: pads, keepouts and existing copper.
    pub fn build(design: &'a Design, rules: &'a ConstraintSet) -> Self {
        let bounds = design.outline.bounds();
        let layers = design
            .layers
            .iter()
            .map(|_| Buckets::new(bounds, BUCKET_SIZE))
            .collect();
        let mut layout = Self {
            design,
            rules,
            outline: Outline::new(&design.outline, rules, BUCKET_SIZE),
            obstacles: Vec::new(),
            layers,
            routed: Vec::new(),
            holes: Vec::new(),
            hole_buckets: Buckets::new(bounds, BUCKET_SIZE),
            widest_hole: 0,
            routed_holes: Vec::new(),
        };
        for pad in &design.pads {
            for copper in &pad.copper {
                layout.insert(copper.layer, Owner::Fixed(pad.net), copper.shape.clone());
            }
        }
        for keepout in &design.keepouts {
            let layers: Vec<LayerId> = match keepout.layer {
                Some(layer) => vec![layer],
                None => design.layer_ids().collect(),
            };
            for layer in layers {
                layout.insert(
                    layer,
                    Owner::Keepout(keepout.scope, keepout.clearance),
                    keepout.shape.clone(),
                );
            }
        }
        for (layer, net, shape) in copper_of(design, &design.existing) {
            layout.insert(layer, Owner::Fixed(Some(net)), shape);
        }
        for via in &design.existing.vias {
            layout.insert_hole(via, None);
        }
        layout
    }

    fn insert(&mut self, layer: LayerId, owner: Owner, shape: Shape) -> usize {
        // A keepout's clearance may exceed the clearance between nets, which
        // bounds a query's reach: its bounds take it in.
        let reach = match owner {
            Owner::Keepout(_, clearance) => clearance,
            _ => 0,
        };
        let bounds = shape.bounds().inflated(reach);
        let id = self.obstacles.len();
        self.obstacles.push(Obstacle {
            owner,
            layer,
            shape,
            bounds,
        });
        self.layers[layer.index()].insert(bounds, id);
        id
    }

    fn obstruction(
        &self,
        net: NetId,
        layer: LayerId,
        shape: &Shape,
        item: Item,
        wanted: Wanted,
    ) -> Obstruction {
        if !self.outline.holds(shape) {
            return Obstruction::Fixed;
        }
        let reach = shape.bounds().inflated(self.rules.largest_clearance());
        let mut routed = Vec::new();
        for id in self.layers[layer.index()].candidates(reach) {
            let obstacle = &self.obstacles[id];
            if !obstacle.bounds.intersects(reach) {
                continue;
            }
            let (connection, clearance) = match obstacle.owner {
                Owner::Fixed(Some(owner)) | Owner::Routed { net: owner, .. } if owner == net => {
                    continue;
                }
                Owner::Fixed(other) => (None, self.rules.clearance(other, Some(net))),
                Owner::Routed {
                    net: other,
                    connection,
                } => (
                    Some(connection),
                    self.rules.clearance(Some(other), Some(net)),
                ),
                Owner::Keepout(KeepoutScope::TracksOnly, _) if item == Item::Via => continue,
                Owner::Keepout(KeepoutScope::ViasOnly, _) if item == Item::Track => continue,
                Owner::Keepout(_, clearance) => (None, clearance),
            };
            if !shape.is_closer_than(&obstacle.shape, clearance) {
                continue;
            }
            match connection {
                None => return Obstruction::Fixed,
                Some(connection) if wanted == Wanted::First => {
                    return Obstruction::Routed(vec![connection]);
                }
                Some(connection) => routed.push(connection),
            }
        }
        obstruction_by(routed)
    }

    fn via_copper(&self, via: &Via) -> impl Iterator<Item = (LayerId, Shape)> + '_ {
        via_copper(self.design, via)
    }

    fn insert_hole(&mut self, via: &Via, connection: Option<ConnectionId>) -> Option<usize> {
        let radius = self.design.via_kind(via.kind).drill? / 2;
        let id = self.holes.len();
        let hole = Hole {
            net: via.net,
            kind: via.kind,
            position: via.position,
            radius,
            connection,
        };
        self.hole_buckets.insert(hole.bounds(), id);
        self.holes.push(hole);
        self.widest_hole = self.widest_hole.max(radius);
        Some(id)
    }

    /// What keeps `via`'s hole the hole-to-hole distance from the others'. A
    /// via may sit on one of its own net: then it is the same hole.
    fn hole_obstruction(&self, via: &Via, wanted: Wanted) -> Obstruction {
        let Some(drill) = self.design.via_kind(via.kind).drill else {
            return Obstruction::None;
        };
        let (radius, apart) = (drill / 2, self.rules.hole_to_hole());
        if apart <= 0 {
            return Obstruction::None;
        }
        let reach = Rect {
            min: via.position,
            max: via.position,
        }
        .inflated(radius + apart + self.widest_hole);
        let mut routed = Vec::new();
        for id in self.hole_buckets.candidates(reach) {
            let hole = &self.holes[id];
            if hole.net == via.net && hole.kind == via.kind && hole.position == via.position {
                continue;
            }
            let gap =
                hole.position.distance(via.position) - i64::from(hole.radius) - i64::from(radius);
            if gap >= i64::from(apart) {
                continue;
            }
            match hole.connection {
                Some(connection) if hole.net != via.net => {
                    if wanted == Wanted::First {
                        return Obstruction::Routed(vec![connection]);
                    }
                    routed.push(connection);
                }
                _ => return Obstruction::Fixed,
            }
        }
        obstruction_by(routed)
    }

    fn via_obstruction_of(&self, via: &Via, wanted: Wanted) -> Obstruction {
        let mut routed = match self.hole_obstruction(via, wanted) {
            Obstruction::None => Vec::new(),
            Obstruction::Fixed => return Obstruction::Fixed,
            Obstruction::Routed(connections) if wanted == Wanted::First => {
                return Obstruction::Routed(connections);
            }
            Obstruction::Routed(connections) => connections,
        };
        for (layer, shape) in self.via_copper(via) {
            match self.obstruction(via.net, layer, &shape, Item::Via, wanted) {
                Obstruction::None => {}
                Obstruction::Fixed => return Obstruction::Fixed,
                Obstruction::Routed(connections) if wanted == Wanted::First => {
                    return Obstruction::Routed(connections);
                }
                Obstruction::Routed(connections) => routed.extend(connections),
            }
        }
        obstruction_by(routed)
    }
}

fn obstruction_by(mut routed: Vec<ConnectionId>) -> Obstruction {
    if routed.is_empty() {
        return Obstruction::None;
    }
    routed.sort_unstable();
    routed.dedup();
    Obstruction::Routed(routed)
}

fn via_copper<'d>(design: &'d Design, via: &Via) -> impl Iterator<Item = (LayerId, Shape)> + 'd {
    let offset = via.position;
    design.via_kind(via.kind).copper.iter().map(move |c| {
        (
            c.layer,
            c.shape.map(|p| Point::new(p.x + offset.x, p.y + offset.y)),
        )
    })
}

/// Every piece of copper of `routes`, with its layer and net.
fn copper_of<'r>(
    design: &'r Design,
    routes: &'r Routes,
) -> impl Iterator<Item = (LayerId, NetId, Shape)> + 'r {
    let tracks = routes.tracks.iter().flat_map(|track| {
        track.path.windows(2).map(move |pair| {
            (
                track.layer,
                track.net,
                Shape::track(Segment::new(pair[0], pair[1]), track.width),
            )
        })
    });
    let vias = routes.vias.iter().flat_map(move |via| {
        via_copper(design, via).map(move |(layer, shape)| (layer, via.net, shape))
    });
    tracks.chain(vias)
}

impl LayoutAccess for Layout<'_> {
    fn track_obstruction(
        &self,
        net: NetId,
        layer: LayerId,
        segment: Segment,
        width: i32,
    ) -> Obstruction {
        self.obstruction(
            net,
            layer,
            &Shape::track(segment, width),
            Item::Track,
            Wanted::All,
        )
    }

    fn via_obstruction(&self, via: &Via) -> Obstruction {
        self.via_obstruction_of(via, Wanted::All)
    }

    fn is_track_clear(&self, net: NetId, layer: LayerId, segment: Segment, width: i32) -> bool {
        self.obstruction(
            net,
            layer,
            &Shape::track(segment, width),
            Item::Track,
            Wanted::First,
        ) == Obstruction::None
    }

    fn is_via_clear(&self, via: &Via) -> bool {
        self.via_obstruction_of(via, Wanted::First) == Obstruction::None
    }

    fn add_route(&mut self, connection: ConnectionId, routes: &Routes) {
        let copper: Vec<_> = copper_of(self.design, routes).collect();
        let ids: Vec<usize> = copper
            .into_iter()
            .map(|(layer, net, shape)| self.insert(layer, Owner::Routed { net, connection }, shape))
            .collect();
        if self.routed.len() <= connection.index() {
            self.routed.resize(connection.index() + 1, Vec::new());
            self.routed_holes.resize(connection.index() + 1, Vec::new());
        }
        self.routed[connection.index()].extend(ids);
        for via in &routes.vias {
            if let Some(id) = self.insert_hole(via, Some(connection)) {
                self.routed_holes[connection.index()].push(id);
            }
        }
    }

    fn remove_route(&mut self, connection: ConnectionId) {
        if let Some(ids) = self.routed.get_mut(connection.index()) {
            for id in ids.drain(..) {
                let obstacle = &self.obstacles[id];
                self.layers[obstacle.layer.index()].remove(obstacle.bounds, id);
            }
        }
        if let Some(ids) = self.routed_holes.get_mut(connection.index()) {
            for id in ids.drain(..) {
                self.hole_buckets.remove(self.holes[id].bounds(), id);
            }
        }
    }
}
