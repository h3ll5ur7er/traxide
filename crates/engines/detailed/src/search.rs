use crate::grid::{DIRECTIONS, Grid, Node};
use crate::history::History;
use crate::lanes::{self, FreeEnd};
use crate::path::{self, Step};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use traxide_contracts::geometry::{Point, Rect, Segment, Shape};
use traxide_contracts::{
    Connection, ConnectionId, ConstraintSet, Corridors, Design, LayerId, LayoutAccess, NetId,
    Obstruction, Pad, PadId, Profile, Routes, Via, ViaKindId,
};
use traxide_scratch::Stamped;

/// No heading yet: just left a pad or a via.
const UNSET: u8 = 8;
/// The slot of the target pad's centre, reached from any node near it.
const TARGET: u32 = u32::MAX;
/// The cost of an edge or via that may not be taken at all.
const BLOCKED: i64 = -1;
/// The best cost of a state once it is expanded: no way to it is cheaper, so it
/// is never expanded again. With a weighted estimate a state is often reached
/// by a worse way first; expanding it again for every better way cost six
/// times the work on long searches, and the path stays within the weight of
/// the cheapest without it.
const CLOSED: i64 = i64::MIN;

/// Search memory kept between connections, so each search costs no allocation.
#[derive(Debug, Default)]
pub(crate) struct Scratch {
    best: Stamped<i64>,
    /// The slot a state was reached from and the via taken, packed (see `Came`).
    came: Stamped<u64>,
    /// How far a state's path has run since the last via it placed, by slot:
    /// a via further back is too far to matter, and the path need not be
    /// walked back to find it.
    since_via: Stamped<i64>,
    /// What crossing obstructions costs on top of an edge's length, or `BLOCKED`.
    edge_extra: Stamped<i64>,
    /// The same for a via at a node.
    via_extra: Stamped<i64>,
    /// What the last stretch to the target's centre costs, by layer and node.
    finish: Stamped<i64>,
    /// Whether nothing is within reach of any edge out of a node, by layer and
    /// node: then no edge out of it needs a check of its own.
    open_around: Stamped<bool>,
    /// How the way through a start state leaves its pad, by state.
    start_stub: Stamped<Stub>,
    /// How the way through a node into the target reaches its pad, by layer and
    /// node.
    finish_stub: Stamped<Stub>,
    /// Every state expanded, in order, for tests to look into a search.
    #[cfg(test)]
    expanded: Vec<u32>,
}

/// A state of the search: a node, a layer and the heading it was entered with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct State {
    node: Node,
    layer: u32,
    heading: u8,
}

#[derive(Debug, Clone, Copy)]
struct Came {
    from: Option<u32>,
    via: Option<ViaKindId>,
}

impl Came {
    fn pack(self) -> u64 {
        let from = self.from.map_or(0, |slot| u64::from(slot) + 1);
        let via = self.via.map_or(0, |kind| u64::from(kind.0) + 1);
        from | via << 32
    }

    fn unpack(packed: u64) -> Self {
        let from = (packed & 0xFFFF_FFFF) as u32;
        let via = (packed >> 32) as u32;
        Self {
            from: from.checked_sub(1),
            via: via.checked_sub(1).map(ViaKindId),
        }
    }
}

/// The area a search for `connection` covers: the corridor global routing
/// planned for it, or the box around its pads grown by `margin` for detours,
/// no further than the board reaches, as no copper lies beyond it.
pub(crate) fn window(
    design: &Design,
    margin: i32,
    connection: Connection,
    corridor: Option<(&Corridors, ConnectionId)>,
) -> Rect {
    let from = design.pad(connection.from).position;
    let pads = Rect {
        min: from,
        max: from,
    }
    .including(design.pad(connection.to).position);
    match corridor.and_then(|(corridors, id)| corridors.bounds(id)) {
        Some(bounds) => bounds.union(pads),
        None => {
            let board = design.outline.bounds().union(pads);
            let grown = |v: i32, by: i32, limit: i32, outwards: fn(i32, i32) -> i32| {
                outwards(v.saturating_add(by), limit)
            };
            Rect {
                min: Point::new(
                    grown(pads.min.x, -margin, board.min.x, i32::max),
                    grown(pads.min.y, -margin, board.min.y, i32::max),
                ),
                max: Point::new(
                    grown(pads.max.x, margin, board.max.x, i32::min),
                    grown(pads.max.y, margin, board.max.y, i32::min),
                ),
            }
        }
    }
}

/// A way onto a pad from a grid node: what reaching the node from the pad
/// costs, and the stub that joins them.
#[derive(Debug, Clone, Copy)]
struct Way {
    state: State,
    cost: i64,
    stub: Stub,
}

/// The track between a pad and the grid: from a point of the pad's copper, the
/// anchor, straight to a node, or along an axis of the pad to a bend and from
/// there onto the grid.
#[derive(Debug, Clone, Copy, Default)]
struct Stub {
    anchor: Point,
    bend: Option<Point>,
}

/// Why a search found no route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NoRoute {
    /// No track can leave these pads at all.
    Blocked {
        /// Pads fixed copper, a keepout or the edge leaves no way out of.
        hemmed_in: Vec<PadId>,
        /// Pads whose every way out the routed copper of other connections took.
        exits_taken: Vec<PadId>,
    },
    NotFound,
}

/// What a search pays to cross routed copper, which negotiated congestion
/// raises where connections keep colliding.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Negotiation<'a> {
    /// How often each connection has been ripped up.
    pub(crate) rip_ups: &'a [u32],
    pub(crate) history: &'a History,
    /// Negotiating in rounds: what overlapping a routed connection costs this
    /// round, for each connection overlapped, which may then stay.
    pub(crate) present: Option<i64>,
}

pub(crate) struct Search<'a, L> {
    design: &'a Design,
    profile: &'a Profile,
    layout: &'a L,
    net: NetId,
    width: i32,
    via_kinds: &'a [ViaKindId],
    hole_to_hole: i32,
    connection: Connection,
    from: &'a Pad,
    to: &'a Pad,
    /// How far from its centre the target is reached, at the free ends of its
    /// lanes, which the estimate must not overshoot.
    to_reach: i64,
    grid: Grid,
    diagonal: i64,
    /// What crossing routed copper costs; `None` when it may not be crossed at all.
    negotiation: Option<Negotiation<'a>>,
    /// Whether routed copper is looked through, to tell what fixed copper alone blocks.
    ignores_routed_copper: bool,
    /// Whether a pad with lanes of the board's wiring is reached only at their
    /// free ends.
    by_lanes_only: bool,
    /// The profile's heuristic weight in thousandths, so ranking stays in integers.
    weight_per_mille: i64,
    /// The corridor global routing planned for this connection, if any.
    corridor: Option<(&'a Corridors, ConnectionId)>,
    /// Copper of the connection's net already joined to its first pad, and
    /// to its second: a way may start on the one and end on the other.
    joined: [&'a [Routes]; 2],
    /// The box around each pad and the copper joined to it, where some is.
    joined_boxes: [Option<Rect>; 2],
}

impl<'a, L: LayoutAccess> Search<'a, L> {
    pub(crate) fn new(
        design: &'a Design,
        rules: &'a ConstraintSet,
        profile: &'a Profile,
        connection: Connection,
        layout: &'a L,
        negotiation: Option<Negotiation<'a>>,
        corridor: Option<(&'a Corridors, ConnectionId)>,
    ) -> Self {
        let (from, to) = (design.pad(connection.from), design.pad(connection.to));
        let window = window(design, profile.detour_margin, connection, corridor);
        let pitch = i64::from(profile.grid_pitch);
        Self {
            connection,
            design,
            profile,
            layout,
            net: connection.net,
            width: rules.track_width(connection.net),
            via_kinds: rules.via_kinds(connection.net),
            hole_to_hole: rules.hole_to_hole(),
            from,
            to,
            to_reach: 0,
            grid: Grid::covering(window, profile.grid_pitch),
            diagonal: Point::new(0, 0)
                .distance(Point::new(profile.grid_pitch, profile.grid_pitch))
                .max(pitch),
            negotiation,
            ignores_routed_copper: false,
            by_lanes_only: false,
            corridor,
            weight_per_mille: (profile.heuristic_weight * 1000.0).round() as i64,
            joined: [&[], &[]],
            joined_boxes: [None, None],
        }
    }

    /// The same search, starting on `from` as well as at the first pad, and
    /// ending on `to` as well as at the second: copper of the net already
    /// joined to each, so that the net grows as a tree.
    pub(crate) fn joining(&self, from: &'a [Routes], to: &'a [Routes]) -> Self {
        let around = |copper: &[Routes], pad: &Pad| {
            (!copper.is_empty())
                .then(|| {
                    Rect::around(
                        (copper.iter())
                            .flat_map(|routes| {
                                (routes.tracks.iter())
                                    .flat_map(|t| t.path.iter().copied())
                                    .chain(routes.vias.iter().map(|v| v.position))
                            })
                            .chain([pad.position]),
                    )
                })
                .flatten()
        };
        Self {
            joined: [from, to],
            joined_boxes: [around(from, self.from), around(to, self.to)],
            ..*self
        }
    }

    /// The same search, looking through routed copper: what it cannot find,
    /// the board's own copper, keepouts and edge leave no way for.
    pub(crate) fn through_routed_copper(&self) -> Self {
        Self {
            ignores_routed_copper: true,
            negotiation: None,
            ..*self
        }
    }

    /// The same search, reaching a pad with lanes only at their free ends,
    /// which then leaves none of them a dangling stub.
    pub(crate) fn by_lanes_only(&self) -> Self {
        Self {
            by_lanes_only: true,
            ..*self
        }
    }

    pub(crate) fn run(self, scratch: &mut Scratch) -> Result<Vec<Step>, NoRoute> {
        let reach = |pad: &Pad| {
            lanes::free_ends(self.design, pad)
                .iter()
                .map(|end| pad.position.distance(end.at))
                .max()
        };
        let (from_reach, to_reach) = (reach(self.from), reach(self.to));
        if from_reach.is_none()
            && to_reach.is_none()
            && self.joined.iter().all(|copper| copper.is_empty())
            && let Some(direct) = self.direct_line()
        {
            return Ok(direct);
        }
        let this = Search {
            to_reach: to_reach.unwrap_or(0),
            ..self
        };
        this.run_both_ways(from_reach.unwrap_or(0), scratch)
    }

    fn run_both_ways(self, from_reach: i64, scratch: &mut Scratch) -> Result<Vec<Step>, NoRoute> {
        let (mut from_access, mut to_access) = (self.access(self.from), self.access(self.to));
        from_access.extend(self.ways_onto_copper(self.joined[0]));
        to_access.extend(self.ways_onto_copper(self.joined[1]));
        if from_access.is_empty() || to_access.is_empty() {
            return Err(self.blocked(from_access.is_empty(), to_access.is_empty()));
        }
        // A search floods everything cheaper than the way into its target. Into a
        // pad hemmed in by its neighbours, such as a pin of a fine-pitch chip,
        // that is most of the board; out of it, only the pocket around it. So
        // the search starts at the end with fewer ways in or out.
        if to_access.len() < from_access.len() {
            let backwards = Search {
                from: self.to,
                to: self.from,
                to_reach: from_reach,
                joined: [self.joined[1], self.joined[0]],
                joined_boxes: [self.joined_boxes[1], self.joined_boxes[0]],
                ..self
            };
            return backwards
                .search(to_access, from_access, scratch)
                .map(path::reversed)
                .ok_or(NoRoute::NotFound);
        }
        self.search(from_access, to_access, scratch)
            .ok_or(NoRoute::NotFound)
    }

    /// Which of the connection's pads with no way out fixed copper hems in, and
    /// which only routed copper does, which may move.
    fn blocked(self, from_blocked: bool, to_blocked: bool) -> NoRoute {
        let fixed_only = self.through_routed_copper();
        let (mut hemmed_in, mut exits_taken) = (Vec::new(), Vec::new());
        for (blocked, id, pad) in [
            (from_blocked, self.connection.from, self.from),
            (to_blocked, self.connection.to, self.to),
        ] {
            match (blocked, fixed_only.access(pad).is_empty()) {
                (false, _) => {}
                (true, true) => hemmed_in.push(id),
                (true, false) => exits_taken.push(id),
            }
        }
        NoRoute::Blocked {
            hemmed_in,
            exits_taken,
        }
    }

    fn search(self, start: Vec<Way>, target: Vec<Way>, scratch: &mut Scratch) -> Option<Vec<Step>> {
        let layers = self.design.layers.len();
        let nodes = self.grid.node_count();
        scratch.best.reset(nodes * layers);
        scratch.came.reset(nodes * layers);
        scratch.since_via.reset(nodes * layers);
        scratch
            .edge_extra
            .reset(nodes * layers * DIRECTIONS.len() / 2);
        scratch.via_extra.reset(nodes * self.via_kinds.len());
        scratch.finish.reset(nodes * layers);
        scratch.open_around.reset(nodes * layers);
        scratch.start_stub.reset(nodes * layers);
        scratch.finish_stub.reset(nodes * layers);
        for Way { state, cost, stub } in target {
            let layer_node = self.layer_node(state);
            if cost < scratch.finish.get(layer_node).unwrap_or(i64::MAX) {
                scratch.finish.set(layer_node, cost);
                scratch.finish_stub.set(layer_node, stub);
            }
        }

        let joins = self.existing_vias_of_the_net();
        let mut best_target = i64::MAX;
        let mut target_came = None;
        let mut open = BinaryHeap::new();
        for Way { state, cost, stub } in start {
            let slot = self.slot(state);
            if cost < scratch.best.get(slot as usize).unwrap_or(i64::MAX) {
                scratch.best.set(slot as usize, cost);
                scratch.start_stub.set(slot as usize, stub);
                scratch.since_via.set(slot as usize, i64::MAX);
                scratch.came.set(
                    slot as usize,
                    Came {
                        from: None,
                        via: None,
                    }
                    .pack(),
                );
                open.push(Reverse((cost + self.ranked_estimate(state), cost, slot)));
            }
        }
        while let Some(Reverse((_, cost, slot))) = open.pop() {
            if slot == TARGET {
                return Some(self.steps(target_came?, scratch));
            }
            if cost > scratch.best.get(slot as usize).unwrap_or(i64::MAX) {
                continue;
            }
            scratch.best.set(slot as usize, CLOSED);
            #[cfg(test)]
            scratch.expanded.push(slot);
            let state = State {
                heading: self.arrival_heading(slot, scratch),
                ..self.state(slot)
            };
            let since_via = scratch.since_via.get(slot as usize).unwrap_or(i64::MAX);
            let mut relax = |next: Option<State>,
                             step_cost: i64,
                             via: Option<ViaKindId>,
                             scratch: &mut Scratch| {
                let total = cost + step_cost;
                let came = Came {
                    from: Some(slot),
                    via,
                };
                match next {
                    Some(next) => {
                        let next_slot = self.slot(next);
                        if total < scratch.best.get(next_slot as usize).unwrap_or(i64::MAX) {
                            scratch.best.set(next_slot as usize, total);
                            scratch.came.set(next_slot as usize, came.pack());
                            let since_via = match (via, next.node == state.node) {
                                (Some(_), _) => 0,
                                (None, true) => since_via,
                                (None, false) => since_via
                                    .saturating_add(self.step_length(state.node, next.node)),
                            };
                            scratch.since_via.set(next_slot as usize, since_via);
                            open.push(Reverse((
                                total + self.ranked_estimate(next),
                                total,
                                next_slot,
                            )));
                        }
                    }
                    None if total < best_target => {
                        best_target = total;
                        target_came = Some(slot);
                        open.push(Reverse((total, total, TARGET)));
                    }
                    None => {}
                }
            };
            if let Some(finish) = scratch.finish.get(self.layer_node(state)) {
                relax(None, finish, None, scratch);
            }
            let is_open_around = scratch
                .open_around
                .get_or_insert_with(self.layer_node(state), || self.is_open_around(state));
            for (direction, (dx, dy)) in DIRECTIONS.iter().enumerate() {
                let turn = turn(state.heading, direction);
                let next = (state.node.0 + dx, state.node.1 + dy);
                // Turns sharper than 90° leave acute angles that trap etchant.
                if turn > 2 || !self.grid.contains(next) || !self.in_corridor(next) {
                    continue;
                }
                let segment = Segment::new(self.grid.point(state.node), self.grid.point(next));
                let extra = if is_open_around {
                    self.negotiation
                        .map_or(0, |n| n.history.of_track(LayerId(state.layer), segment))
                } else {
                    // An edge is the same both ways: kept once, at the node it
                    // leaves in one of the first half of the directions.
                    let half = DIRECTIONS.len() / 2;
                    let edge = if direction < half {
                        self.layer_node(state) * half + direction
                    } else {
                        let from = State {
                            node: next,
                            ..state
                        };
                        self.layer_node(from) * half + direction - half
                    };
                    scratch.edge_extra.get_or_insert_with(edge, || {
                        self.track_extra(LayerId(state.layer), segment)
                            .unwrap_or(BLOCKED)
                    })
                };
                if extra != BLOCKED {
                    let length = if dx * dy == 0 {
                        i64::from(self.profile.grid_pitch)
                    } else {
                        self.diagonal
                    };
                    let next = State {
                        node: next,
                        layer: state.layer,
                        heading: direction as u8,
                    };
                    let cost = length + i64::from(turn) * self.profile.bend_cost + extra;
                    relax(Some(next), cost, None, scratch);
                }
            }
            for (position, &kind) in self.via_kinds.iter().enumerate() {
                if self.is_near_a_via_of_its_path(slot, kind, scratch) {
                    continue;
                }
                let spans = self.design.via_kind(kind);
                let others = self.design.layer_ids().filter(|&l| {
                    l.0 != state.layer
                        && self.design.layers[l.index()].routable
                        && spans.connects(LayerId(state.layer), l)
                });
                for layer in others.collect::<Vec<_>>() {
                    let check = self.grid.index(state.node) * self.via_kinds.len() + position;
                    let extra = scratch.via_extra.get_or_insert_with(check, || {
                        let via = Via {
                            net: self.net,
                            kind,
                            position: self.grid.point(state.node),
                        };
                        self.via_extra(&via).unwrap_or(BLOCKED)
                    });
                    if extra != BLOCKED {
                        let next = State {
                            node: state.node,
                            layer: layer.0,
                            heading: UNSET,
                        };
                        relax(
                            Some(next),
                            self.profile.via_cost + extra,
                            Some(kind),
                            scratch,
                        );
                    }
                }
            }
            // At a via's price, though the via is there: for free, paths
            // crowd through the board's vias and take each other's room.
            self.relax_through_existing_vias(state, &joins, |next| {
                relax(Some(next), self.profile.via_cost, None, scratch);
            });
        }
        None
    }

    /// The length of the step between two neighbouring nodes of the grid.
    fn step_length(&self, from: Node, to: Node) -> i64 {
        if from.0 == to.0 || from.1 == to.1 {
            i64::from(self.profile.grid_pitch)
        } else {
            self.diagonal
        }
    }

    /// Whether a via of `kind` at `slot`'s node would sit closer than the
    /// hole-to-hole distance to a via the path to it placed, which the layout
    /// does not hold yet. The path is walked back only as far as such a via
    /// could be.
    fn is_near_a_via_of_its_path(&self, slot: u32, kind: ViaKindId, scratch: &Scratch) -> bool {
        let apart = i64::from(self.hole_to_hole);
        let Some(drill) = self.design.via_kind(kind).drill.map(i64::from) else {
            return false;
        };
        if apart <= 0 {
            return false;
        }
        let reach = apart + 2 * drill;
        // The walk below looks at the node one step beyond `reach` too.
        let since_via = scratch.since_via.get(slot as usize).unwrap_or(i64::MAX);
        if since_via > reach + self.diagonal {
            return false;
        }
        let here = self.grid.point(self.state(slot).node);
        let (mut current, mut previous, mut walked) = (slot, here, 0);
        while walked <= reach {
            let Some(packed) = scratch.came.get(current as usize) else {
                return false;
            };
            let came = Came::unpack(packed);
            let at = self.grid.point(self.state(current).node);
            walked += previous.distance(at);
            previous = at;
            let other = came.via.and_then(|other| self.design.via_kind(other).drill);
            if let Some(other) = other {
                let gap = here.distance(at) - drill / 2 - i64::from(other) / 2;
                if gap < apart {
                    return true;
                }
            }
            match came.from {
                Some(from) => current = from,
                None => return false,
            }
        }
        false
    }

    /// The copper of the vias of this net the board came with that lie in the
    /// search's window: as they stay, a path may change layers through one
    /// instead of placing a via of its own beside it, which would sit closer
    /// to it than the hole-to-hole distance.
    fn existing_vias_of_the_net(&self) -> Vec<Vec<(LayerId, Shape)>> {
        self.design
            .existing
            .vias
            .iter()
            .filter(|via| via.net == self.net && self.grid.covers(via.position))
            .map(|via| {
                self.design
                    .via_kind(via.kind)
                    .copper
                    .iter()
                    .map(|c| (c.layer, c.shape.map(|p| p + via.position)))
                    .collect()
            })
            .collect()
    }

    /// Hands `relax` the states on the other layers of each existing via whose
    /// copper covers `state`'s node on its layer and theirs.
    fn relax_through_existing_vias(
        &self,
        state: State,
        joins: &[Vec<(LayerId, Shape)>],
        mut relax: impl FnMut(State),
    ) {
        let here = self.grid.point(state.node);
        for copper in joins {
            let covers = |layer: LayerId| {
                copper
                    .iter()
                    .any(|(l, shape)| *l == layer && shape.nearest_point(here) == here)
            };
            if !covers(LayerId(state.layer)) {
                continue;
            }
            for layer in self.design.layer_ids() {
                if layer.0 != state.layer
                    && self.design.layers[layer.index()].routable
                    && covers(layer)
                {
                    relax(State {
                        node: state.node,
                        layer: layer.0,
                        heading: UNSET,
                    });
                }
            }
        }
    }

    /// What a track segment costs beyond its length: nothing when it is clear,
    /// the price of ripping up what it crosses when that is allowed, else `None`.
    fn track_extra(&self, layer: LayerId, segment: Segment) -> Option<i64> {
        if self.ignores_routed_copper {
            let obstruction = self
                .layout
                .track_obstruction(self.net, layer, segment, self.width);
            return (obstruction != Obstruction::Fixed).then_some(0);
        }
        match self.negotiation {
            None => self
                .layout
                .is_track_clear(self.net, layer, segment, self.width)
                .then_some(0),
            Some(negotiation) => {
                let obstruction = self
                    .layout
                    .track_obstruction(self.net, layer, segment, self.width);
                Some(
                    self.rip_up_price(negotiation, obstruction)?
                        + negotiation.history.of_track(layer, segment),
                )
            }
        }
    }

    fn via_extra(&self, via: &Via) -> Option<i64> {
        if self.ignores_routed_copper {
            return (self.layout.via_obstruction(via) != Obstruction::Fixed).then_some(0);
        }
        match self.negotiation {
            None => self.layout.is_via_clear(via).then_some(0),
            Some(negotiation) => {
                let layers = self
                    .design
                    .via_kind(via.kind)
                    .copper
                    .iter()
                    .map(|c| c.layer);
                Some(
                    self.rip_up_price(negotiation, self.layout.via_obstruction(via))?
                        + negotiation.history.of_via(via, layers),
                )
            }
        }
    }

    /// Each crossed connection costs more the more often it was ripped up before,
    /// so that connections settle instead of taking turns (negotiated congestion's
    /// history cost); one ripped up too often may not be crossed again.
    fn rip_up_price(&self, negotiation: Negotiation, obstruction: Obstruction) -> Option<i64> {
        match obstruction {
            Obstruction::None => Some(0),
            Obstruction::Fixed => None,
            Obstruction::Routed(connections) if negotiation.present.is_some() => {
                Some(negotiation.present? * connections.len() as i64)
            }
            Obstruction::Routed(connections) => {
                let rip_ups = negotiation.rip_ups;
                connections
                    .iter()
                    .map(|c| {
                        let times = rip_ups[c.index()];
                        (times < self.profile.max_rip_ups)
                            .then(|| self.profile.rip_up_cost * i64::from(times + 1))
                    })
                    .sum()
            }
        }
    }

    /// Whether no copper, keepout or edge is within reach of a track along any
    /// edge out of `state`'s node: one check of a disk that covers them all,
    /// instead of eight. Most nodes of a search lie in open space.
    fn is_open_around(&self, state: State) -> bool {
        let here = self.grid.point(state.node);
        let reach = i32::try_from(self.diagonal).unwrap_or(i32::MAX / 4);
        self.layout.is_track_clear(
            self.net,
            LayerId(state.layer),
            Segment::new(here, here),
            self.width + 2 * reach,
        )
    }

    fn in_corridor(&self, node: Node) -> bool {
        self.corridor
            .is_none_or(|(corridors, id)| corridors.allows(id, self.grid.point(node)))
    }

    fn layer_node(&self, state: State) -> usize {
        state.layer as usize * self.grid.node_count() + self.grid.index(state.node)
    }

    /// A position of the search: a node on a layer. Its heading is not part of
    /// it: the nine ways into a node cost nearly the same, as a bend costs
    /// little, and searching them apart expanded nearly every node nine times.
    fn slot(&self, state: State) -> u32 {
        u32::try_from(self.layer_node(state)).expect("search windows below 2³² states")
    }

    /// The node and layer of `slot`, without a heading.
    fn state(&self, slot: u32) -> State {
        let layer_node = slot as usize;
        let nodes = self.grid.node_count();
        State {
            node: self.grid.node(layer_node % nodes),
            layer: (layer_node / nodes) as u32,
            heading: UNSET,
        }
    }

    /// The heading of the cheapest way into `slot` found so far: the direction
    /// of its last step, or none after a via or at a pad.
    fn arrival_heading(&self, slot: u32, scratch: &Scratch) -> u8 {
        let came = Came::unpack(scratch.came.get(slot as usize).unwrap_or_default());
        let (Some(from), None) = (came.from, came.via) else {
            return UNSET;
        };
        let (here, there) = (self.state(slot).node, self.state(from).node);
        DIRECTIONS
            .iter()
            .position(|&d| d == (here.0 - there.0, here.1 - there.1))
            .map_or(UNSET, |d| d as u8)
    }

    /// The pad centres joined without a search, when they see each other on a
    /// layer both share: straight if they line up at a multiple of 45°, else one
    /// straight and one diagonal segment, as octilinear routing requires.
    fn direct_line(&self) -> Option<Vec<Step>> {
        let candidates = octilinear_paths(self.from.position, self.to.position);
        let fits = |layer: LayerId, path: &[Point]| {
            path.windows(2).all(|pair| {
                let segment = Segment::new(pair[0], pair[1]);
                self.layout
                    .is_track_clear(self.net, layer, segment, self.width)
            })
        };
        self.routable_layers(self.from)
            .filter(|&layer| self.to.is_on(layer))
            .find_map(|layer| {
                let path = candidates.iter().find(|path| fits(layer, path))?;
                Some(
                    path.iter()
                        .map(|&at| Step {
                            at,
                            layer,
                            via: None,
                        })
                        .collect(),
                )
            })
    }

    fn routable_layers<'p>(&'p self, pad: &'p Pad) -> impl Iterator<Item = LayerId> + 'p {
        pad.copper
            .iter()
            .map(|c| c.layer)
            .filter(|l| self.design.layers[l.index()].routable)
    }

    /// The ways from grid nodes onto `pad`: straight to its centre, the pin's
    /// position, where that is clear, else to the nearest point of its copper,
    /// as the centre of an edge connector's pad may lie off the board; and
    /// along the pad's axes, for a centre off the grid.
    ///
    /// A pad with a lane of the board's wiring is reached at the lane's free
    /// end too, which leaves no dangling stub; searching by lanes only, there
    /// alone where the lanes leave a way in.
    fn access(&self, pad: &Pad) -> Vec<Way> {
        let into_lanes: Vec<Way> = lanes::free_ends(self.design, pad)
            .iter()
            .flat_map(|end| self.ways_into_free_end(end))
            .collect();
        if self.by_lanes_only && !into_lanes.is_empty() {
            return into_lanes;
        }
        let mut ways = self.ways_onto(pad);
        ways.extend(into_lanes);
        ways
    }

    fn ways_into_free_end(&self, end: &FreeEnd) -> Vec<Way> {
        let around = Rect {
            min: end.at,
            max: end.at,
        };
        let mut ways = Vec::new();
        for &layer in &end.layers {
            if !self.design.layers[layer.index()].routable {
                continue;
            }
            for node in self
                .grid
                .nodes_near(around, end.at)
                .into_iter()
                .filter(|&n| self.in_corridor(n))
            {
                let point = self.grid.point(node);
                if let Some(extra) = self.track_extra(layer, Segment::new(end.at, point)) {
                    ways.push(Way {
                        state: State {
                            node,
                            layer: layer.0,
                            heading: UNSET,
                        },
                        cost: end.at.distance(point) + extra,
                        stub: Stub {
                            anchor: end.at,
                            bend: None,
                        },
                    });
                }
            }
        }
        ways
    }

    /// The ways that start or end right on `copper`: the nodes on its tracks
    /// and under its vias, on each layer they lie on.
    fn ways_onto_copper(&self, copper: &[Routes]) -> Vec<Way> {
        let pitch = self.profile.grid_pitch;
        let node_at = |point: Point| {
            (point.x % pitch == 0 && point.y % pitch == 0)
                .then(|| (point.x / pitch, point.y / pitch))
        };
        let way = |node: Node, layer: LayerId| Way {
            state: State {
                node,
                layer: layer.0,
                heading: UNSET,
            },
            cost: 0,
            stub: Stub {
                anchor: self.grid.point(node),
                bend: None,
            },
        };
        let mut ways = Vec::new();
        for routes in copper {
            for track in &routes.tracks {
                for pair in track.path.windows(2) {
                    let (Some(a), Some(b)) = (node_at(pair[0]), node_at(pair[1])) else {
                        continue;
                    };
                    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                    if !(dx == 0 || dy == 0 || dx.abs() == dy.abs()) {
                        continue;
                    }
                    let steps = dx.abs().max(dy.abs());
                    ways.extend(
                        (0..=steps)
                            .map(|k| (a.0 + k * dx.signum(), a.1 + k * dy.signum()))
                            .filter(|&node| self.grid.contains(node) && self.in_corridor(node))
                            .map(|node| way(node, track.layer)),
                    );
                }
            }
            for via in &routes.vias {
                let Some(node) = node_at(via.position).filter(|&n| self.grid.contains(n)) else {
                    continue;
                };
                ways.extend(
                    (self.design.via_kind(via.kind).copper.iter())
                        .filter(|c| self.design.layers[c.layer.index()].routable)
                        .map(|c| way(node, c.layer)),
                );
            }
        }
        ways
    }

    fn ways_onto(&self, pad: &Pad) -> Vec<Way> {
        let mut ways = Vec::new();
        for copper in pad
            .copper
            .iter()
            .filter(|c| self.design.layers[c.layer.index()].routable)
        {
            for node in self
                .grid
                .nodes_near(copper.shape.bounds(), pad.position)
                .into_iter()
                .filter(|&n| self.in_corridor(n))
            {
                let point = self.grid.point(node);
                let state = State {
                    node,
                    layer: copper.layer.0,
                    heading: UNSET,
                };
                let nearest = copper.shape.nearest_point(point);
                let way = [pad.position, nearest].into_iter().find_map(|anchor| {
                    let extra = self.track_extra(copper.layer, Segment::new(anchor, point))?;
                    Some(Way {
                        state,
                        cost: anchor.distance(point) + extra,
                        stub: Stub { anchor, bend: None },
                    })
                });
                ways.extend(way);
            }
            for axis_way in DIRECTIONS
                .iter()
                .flat_map(|&direction| self.ways_along(pad.position, copper.layer, direction))
            {
                match ways.iter_mut().find(|w| w.state == axis_way.state) {
                    Some(way) if way.cost <= axis_way.cost => {}
                    Some(way) => *way = axis_way,
                    None => ways.push(axis_way),
                }
            }
        }
        ways
    }

    /// The ways that leave `centre` straight along `direction` and turn onto
    /// the grid where the stub first crosses a grid line beyond the copper
    /// around it. Between the neighbours of a fine-pitch pin a track fits only
    /// on the pin's axis, which no grid line follows when the pin lies off the
    /// grid; so the stub runs off the grid, and only its short jog, square to a
    /// grid line, joins the grid.
    fn ways_along(&self, centre: Point, layer: LayerId, direction: (i32, i32)) -> Vec<Way> {
        let pitch = self.profile.grid_pitch;
        for bend in grid_crossings(centre, direction, pitch) {
            if !self.grid.covers(bend) {
                break;
            }
            let Some(stub_extra) = self.track_extra(layer, Segment::new(centre, bend)) else {
                break;
            };
            let ways: Vec<Way> = jogs(bend, direction, pitch)
                .into_iter()
                .filter(|&node| self.grid.contains(node) && self.in_corridor(node))
                .filter_map(|node| {
                    let point = self.grid.point(node);
                    let jog_extra = if point == bend {
                        0
                    } else {
                        self.track_extra(layer, Segment::new(bend, point))?
                    };
                    Some(Way {
                        state: State {
                            node,
                            layer: layer.0,
                            heading: UNSET,
                        },
                        cost: centre.distance(bend) + bend.distance(point) + stub_extra + jog_extra,
                        stub: Stub {
                            anchor: centre,
                            bend: (point != bend).then_some(bend),
                        },
                    })
                })
                .collect();
            if !ways.is_empty() {
                return ways;
            }
        }
        Vec::new()
    }

    /// What is left to pay, as if nothing were in the way: the octilinear distance
    /// to the target and a via if the target has no copper on this layer. Paths
    /// on the grid are octilinear, so this is far tighter than the straight
    /// distance; only the short any-angle stub into the pad can cost less.
    fn estimate(&self, state: State) -> i64 {
        let here = self.grid.point(state.node);
        if let Some(target) = self.joined_boxes[1] {
            // The way may end on any of the copper joined to the target, on
            // any layer: the distance to the box around it all is the least.
            let outside = |v: i32, min: i32, max: i32| i64::from((min - v).max(v - max).max(0));
            let dx = outside(here.x, target.min.x, target.max.x);
            let dy = outside(here.y, target.min.y, target.max.y);
            let (long, short) = (dx.max(dy), dx.min(dy));
            return long - short + short * self.diagonal / i64::from(self.profile.grid_pitch);
        }
        let dx = i64::from(here.x.abs_diff(self.to.position.x));
        let dy = i64::from(here.y.abs_diff(self.to.position.y));
        let (long, short) = (dx.max(dy), dx.min(dy));
        let octilinear = long - short + short * self.diagonal / i64::from(self.profile.grid_pitch);
        if self.to_reach > 0 {
            // A lane may end on any layer, so no via is certain.
            return (octilinear - self.to_reach).max(0);
        }
        let via = if self.to.is_on(LayerId(state.layer)) {
            0
        } else {
            self.profile.via_cost
        };
        octilinear + via
    }

    /// The estimate as the search ranks states: weighted by the profile, so the
    /// search heads for the target more boldly than the cheapest path requires.
    fn ranked_estimate(&self, state: State) -> i64 {
        self.estimate(state) * self.weight_per_mille / 1000
    }

    fn steps(&self, last: u32, scratch: &Scratch) -> Vec<Step> {
        let last_state = self.state(last);
        let last_layer = LayerId(last_state.layer);
        let finish = scratch
            .finish_stub
            .get(self.layer_node(last_state))
            .expect("the target is reached through a way into it");
        let mut reversed: Vec<Step> = stub_points_into_pad(finish)
            .map(|at| Step {
                at,
                layer: last_layer,
                via: None,
            })
            .collect();
        let mut first = last;
        let mut current = Some(last);
        while let Some(slot) = current {
            first = slot;
            let state = self.state(slot);
            let came = Came::unpack(
                scratch
                    .came
                    .get(slot as usize)
                    .expect("every reached state knows its origin"),
            );
            reversed.push(Step {
                at: self.grid.point(state.node),
                layer: LayerId(state.layer),
                via: came.via,
            });
            current = came.from;
        }
        let first_layer = reversed.last().map_or(LayerId(0), |s| s.layer);
        let start = scratch
            .start_stub
            .get(first as usize)
            .expect("every search starts from a way out of its pad");
        reversed.extend(stub_points(start).map(|at| Step {
            at,
            layer: first_layer,
            via: None,
        }));
        reversed.reverse();
        reversed
    }
}

/// The octilinear ways from `from` to `to` with at most one bend: diagonal
/// first, then straight first.
fn octilinear_paths(from: Point, to: Point) -> Vec<Vec<Point>> {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    if dx == 0 || dy == 0 || dx.abs() == dy.abs() {
        return vec![vec![from, to]];
    }
    let diagonal = dx.abs().min(dy.abs());
    let step = Point::new(dx.signum() * diagonal, dy.signum() * diagonal);
    vec![vec![from, from + step, to], vec![from, to - step, to]]
}

/// The points of a stub from its pad outwards, without the node it reaches,
/// in the order a path walked backwards meets them at its start.
fn stub_points(stub: Stub) -> impl Iterator<Item = Point> {
    stub.bend.into_iter().chain([stub.anchor])
}

/// The points of a stub into its pad, in the order a path walked backwards
/// meets them at its end: the pad first.
fn stub_points_into_pad(stub: Stub) -> impl Iterator<Item = Point> {
    [stub.anchor].into_iter().chain(stub.bend)
}

/// The points where a ray from `from` along `direction` crosses a grid line of
/// `pitch`, nearest first.
fn grid_crossings(from: Point, (dx, dy): (i32, i32), pitch: i32) -> impl Iterator<Item = Point> {
    let first = |v: i32, d: i32| {
        (d != 0).then(|| match (-v * d).rem_euclid(pitch) {
            0 => pitch,
            t => t,
        })
    };
    let mut firsts = [first(from.x, dx), first(from.y, dy)];
    firsts.sort_unstable();
    if firsts[0] == firsts[1] {
        firsts[0] = None;
    }
    (0..).flat_map(move |k| {
        firsts
            .into_iter()
            .flatten()
            .map(move |t| t + k * pitch)
            .map(move |t| Point::new(from.x + t * dx, from.y + t * dy))
    })
}

/// The nodes a stub that ran along `direction` to `bend`, on a grid line, may
/// jog to: `bend` itself if it is a node, else the nearest node on either side
/// along that grid line, but never back against the stub, as a turn sharper
/// than 90° leaves an acute angle that traps etchant.
fn jogs(bend: Point, (dx, dy): (i32, i32), pitch: i32) -> Vec<Node> {
    let lower = (bend.x.div_euclid(pitch), bend.y.div_euclid(pitch));
    let (upper, onwards) = match (bend.x.rem_euclid(pitch), bend.y.rem_euclid(pitch)) {
        (0, 0) => return vec![lower],
        (_, 0) => ((lower.0 + 1, lower.1), dx),
        _ => ((lower.0, lower.1 + 1), dy),
    };
    [(lower, -1), (upper, 1)]
        .into_iter()
        .filter(|&(_, side)| side * onwards >= 0)
        .map(|(node, _)| node)
        .collect()
}

/// How many 45° steps lie between a heading and the next direction.
fn turn(heading: u8, direction: usize) -> u8 {
    if heading == UNSET {
        return 0;
    }
    let difference = (i16::from(heading) - direction as i16).rem_euclid(8) as u8;
    difference.min(8 - difference)
}

#[cfg(test)]
mod tests {
    use super::*;
    use traxide_contracts::geometry::{Polygon, Shape};
    use traxide_contracts::testing::{DesignBuilder, at};
    use traxide_contracts::{KeepoutScope, Plan};
    use traxide_rules::Purpose;

    fn wall(min: Point, max: Point) -> Shape {
        Shape::Polygon(Polygon::rectangle(Rect { min, max }))
    }

    #[test]
    fn a_ray_crosses_the_grid_lines_nearest_first() {
        let crossings: Vec<Point> = grid_crossings(Point::new(30, 70), (1, 1), 100)
            .take(4)
            .collect();
        assert_eq!(
            crossings,
            [
                Point::new(60, 100),
                Point::new(100, 140),
                Point::new(160, 200),
                Point::new(200, 240)
            ]
        );
    }

    #[test]
    fn a_stub_jogs_onto_the_grid_square_to_it_and_never_back() {
        assert_eq!(jogs(Point::new(30, 200), (0, 1), 100), [(0, 2), (1, 2)]);
        assert_eq!(jogs(Point::new(100, 230), (1, 1), 100), [(1, 3)]);
        assert_eq!(jogs(Point::new(100, 200), (1, 1), 100), [(1, 2)]);
    }

    #[test]
    fn a_weighted_search_expands_no_state_twice() {
        // The start sits in a cup that opens away from the target: a search
        // heading boldly for the target fills the cup, reaching many states
        // first by a worse way round.
        let design = DesignBuilder::new(30.0, 20.0)
            .layers(1)
            .without_vias()
            .keepout(
                wall(at(8.0, 5.0), at(9.0, 15.0)),
                None,
                KeepoutScope::Everything,
            )
            .keepout(
                wall(at(3.0, 5.0), at(9.0, 6.0)),
                None,
                KeepoutScope::Everything,
            )
            .keepout(
                wall(at(3.0, 14.0), at(9.0, 15.0)),
                None,
                KeepoutScope::Everything,
            )
            .smd("A1", Some("A"), at(6.0, 10.0), 0.5, 0)
            .smd("A2", Some("A"), at(25.0, 10.0), 0.5, 0)
            .build();
        let rules = traxide_rules::compile(
            &design,
            Purpose::Routing {
                grid_pitch: 100_000,
            },
        );
        let layout = traxide_layout_access::Layout::build(&design, &rules);
        let profile = Profile {
            name: "bold".into(),
            grid_pitch: 100_000,
            via_cost: 2_000_000,
            bend_cost: 100_000,
            detour_margin: 10_000_000,
            rip_up_cost: 5_000_000,
            max_rip_ups: 0,
            history_cost: 0,
            heuristic_weight: 2.0,
            global_routing: false,
            batch_size: 1,
        };
        let plan = Plan {
            connections: vec![Connection {
                net: design.net_named("A"),
                from: design.pad_named("A1"),
                to: design.pad_named("A2"),
            }],
        };
        let mut scratch = Scratch::default();
        let search = Search::new(
            &design,
            &rules,
            &profile,
            plan.connections[0],
            &layout,
            None,
            None,
        );
        assert!(search.run(&mut scratch).is_ok());

        let mut distinct = scratch.expanded.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(scratch.expanded.len(), distinct.len());
    }
}
