//! Detailed Routing Engine: the algorithm that turns connections into exact,
//! rule-correct copper (volatility V7 of docs/Architecture/Volatilities.md).
//!
//! For now an A* search on an octilinear grid across all routable layers, in
//! batches of connections far enough apart to be searched side by side, with
//! rip-up and reroute: a connection that finds its way blocked by routed
//! copper may take that copper's place, and the connections it displaces are
//! routed again later.

mod grid;
mod history;
mod lanes;
mod path;
mod search;

use search::NoRoute;
use std::cmp::Reverse;
use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use traxide_contracts::geometry::{Point, Rect, Segment};
use traxide_contracts::{
    Connection, ConnectionId, ConstraintSet, Corridors, Design, Event, LayoutAccess, NetId,
    Obstruction, Plan, Profile, Routes, RoutingResult,
};
use traxide_events::{CancelToken, Publisher};

/// How many connections a batch looks through for each connection it may hold.
const SCANNED_PER_BATCH_SLOT: usize = 4;

/// Measured on the corpus: one round left Issue034 3 connections short of
/// complete. Two cost the Mars-64 boards 6 connections until fine-pitch pins
/// escaped before routing; since, two route as many boards completely as
/// four and 4 connections more in all, in a tenth less time, half on the
/// churning boards (TRX-56).
const STALLED_ROUNDS: usize = 2;

/// Negotiating in rounds ends after this many rounds without fewer overlaps.
const ROUNDS_WITHOUT_GAIN: usize = 4;

/// How much wider each window of a search that found no way grows.
const WIDER_WINDOW: i32 = 4;

/// How many times dearer a via is when routing is tidied: negotiating at that
/// price costs connections and time, but once everything is routed a
/// connection may well find a way with fewer vias among the rest (TRX-72).
const TIDYING_VIA_COST: i64 = 4;

/// How boldly a search heads for its target when routing is tidied: any way
/// with fewer vias will do, and the profile's weight, searching for the
/// cheapest, made tidying take as long as routing on small boards.
const TIDYING_HEURISTIC_WEIGHT: f64 = 2.0;

pub struct Router<'a> {
    finder: Finder<'a>,
    /// One per thread: each search needs its own memory.
    scratches: Vec<search::Scratch>,
    /// When negotiation stops ripping up, whatever it would still gain.
    deadline: Option<Instant>,
}

impl std::fmt::Debug for Router<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field("profile", &self.finder.profile.name)
            .field("threads", &self.scratches.len())
            .finish_non_exhaustive()
    }
}

impl<'a> Router<'a> {
    pub fn new(design: &'a Design, rules: &'a ConstraintSet, profile: &'a Profile) -> Self {
        Self {
            finder: Finder {
                design,
                rules,
                profile,
            },
            scratches: vec![search::Scratch::default()],
            deadline: None,
        }
    }

    /// Searches the connections of a batch on up to `threads` threads. The
    /// routes are the same on any number of them.
    /// Stops negotiation at `deadline`: what is still queued then routes once
    /// more without crossing anything, each in its own window only, so routing
    /// ends a little later.
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    fn is_past_deadline(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    pub fn with_threads(mut self, threads: usize) -> Self {
        self.scratches
            .resize_with(threads.max(1), search::Scratch::default);
        self
    }

    /// Routes the connections of `plan` in order, adding their copper to `layout`.
    /// Connections that cannot touch each other's copper are searched side by
    /// side in batches and committed in order (see `next_batch`).
    /// A connection is ripped up at most the profile's `max_rip_ups` times; those
    /// that find no way within that are reported as unrouted. Once
    /// STALLED_ROUNDS rounds of rip-ups, as many as there are connections each,
    /// have routed no more connections than before, negotiation is going round
    /// in circles: the connections left search without ripping up any other.
    ///
    /// Where that leaves connections unrouted, the board is routed a second
    /// time, settled in rounds first (`settle_in_rounds`), and the attempt that
    /// completes more nets is kept (`Attempt::standing`): rounds route more on
    /// congested boards but lose a
    /// connection on some that rip-up completes, which never pay for them (TRX-56).
    pub fn route_plan(
        &mut self,
        plan: &Plan,
        corridors: Option<&Corridors>,
        layout: &mut (impl LayoutAccess + Sync),
        cancel: &CancelToken,
        events: &impl Publisher<Event>,
    ) -> RoutingResult {
        let first = self.negotiate(self.fresh(plan), plan, corridors, layout, cancel, events);
        let mut best = if first.unrouted.is_empty()
            || self.finder.profile.max_rip_ups == 0
            || cancel.is_cancelled()
            || self.is_past_deadline()
        {
            first
        } else {
            first.take_out(layout);
            let mut state = self.fresh(plan);
            self.settle_in_rounds(&mut state, plan, corridors, layout, cancel, events);
            let second = self.negotiate(state, plan, corridors, layout, cancel, events);
            if second.standing(plan) > first.standing(plan) {
                second
            } else {
                second.take_out(layout);
                first.put_back(layout);
                first
            }
        };
        if !cancel.is_cancelled() && !self.is_past_deadline() {
            self.tidy(&mut best, plan, layout, cancel);
        }
        best.into_result(plan)
    }

    /// The vias a connection needs at least: one between pads that share no
    /// layer tracks may run on.
    fn fewest_vias(&self, connection: Connection) -> usize {
        let design = self.finder.design;
        let (from, to) = (design.pad(connection.from), design.pad(connection.to));
        let shared = from
            .copper
            .iter()
            .any(|c| design.layers[c.layer.index()].routable && to.is_on(c.layer));
        usize::from(!shared)
    }

    /// Routes each routed connection with more vias than it needs once more,
    /// among the rest without crossing them, with vias TIDYING_VIA_COST times
    /// dearer, and keeps the route that has fewer vias. One at a time: batches
    /// keep apart what connections may search, not what they routed, and a
    /// route found in a wider window (TRX-55) reaches beyond that.
    fn tidy(
        &mut self,
        attempt: &mut Attempt,
        plan: &Plan,
        layout: &mut (impl LayoutAccess + Sync),
        cancel: &CancelToken,
    ) {
        let dearer = Profile {
            via_cost: self.finder.profile.via_cost * TIDYING_VIA_COST,
            heuristic_weight: TIDYING_HEURISTIC_WEIGHT,
            ..self.finder.profile.clone()
        };
        let finder = Finder {
            profile: &dearer,
            ..self.finder
        };
        for (i, routed) in attempt.routed.iter_mut().enumerate() {
            if cancel.is_cancelled() || self.is_past_deadline() {
                return;
            }
            let connection = plan.connections[i];
            let Some(routes) = routed else { continue };
            if routes.vias.len() <= self.fewest_vias(connection) {
                continue;
            }
            let id = ConnectionId::from_index(i);
            layout.remove_route(id);
            let tidier = finder
                .search(&mut self.scratches[0], connection, layout, None, None)
                .ok()
                .filter(|tidier| tidier.vias.len() < routes.vias.len());
            if let Some(tidier) = tidier {
                *routes = tidier;
            }
            layout.add_route(id, routes);
        }
    }

    fn fresh(&self, plan: &Plan) -> Negotiated {
        let total = plan.connections.len();
        Negotiated {
            routed: vec![None; total],
            rip_ups: vec![0; total],
            queue: (0..total).map(ConnectionId::from_index).collect(),
            history: history::History::new(self.finder.design, self.finder.profile.history_cost),
            may_rip_up: self.finder.profile.max_rip_ups > 0,
            most_routed: 0,
            rip_ups_since_most: 0,
        }
    }

    /// Rip-up and reroute of what `state` holds queued, until nothing is.
    fn negotiate(
        &mut self,
        mut state: Negotiated,
        plan: &Plan,
        corridors: Option<&Corridors>,
        layout: &mut (impl LayoutAccess + Sync),
        cancel: &CancelToken,
        events: &impl Publisher<Event>,
    ) -> Attempt {
        let mut unrouted = Vec::new();
        let (mut hemmed_in, mut exits_taken, mut cut_off) = (Vec::new(), Vec::new(), Vec::new());
        while !state.queue.is_empty() {
            if cancel.is_cancelled() {
                unrouted.extend(state.queue.drain(..));
                break;
            }
            if self.is_past_deadline() {
                state.may_rip_up = false;
            }
            let batch = self.next_batch(&mut state.queue, plan, corridors);
            let found = self.find_all(&batch, plan, corridors, layout, state.negotiation());
            let mut no_way = Vec::new();
            for (id, found) in batch.iter().copied().zip(found) {
                match found {
                    Ok(found) => {
                        state.commit(self.finder.design, layout, id, found, &batch, events)
                    }
                    Err(NoRoute::NotFound) => no_way.push(id),
                    Err(NoRoute::Blocked {
                        hemmed_in: fixed,
                        exits_taken: taken,
                    }) => {
                        hemmed_in.extend(fixed);
                        exits_taken.extend(taken);
                        unrouted.push(id);
                    }
                }
            }
            // After the batch, one at a time, so that a wider window cannot meet
            // the copper of a connection searched beside it.
            for id in no_way {
                let connection = plan.connections[id.index()];
                // Past the deadline no time is left for searches up to the whole board.
                if self.is_past_deadline() {
                    unrouted.push(id);
                    continue;
                }
                match self.find_wider(connection, layout, state.negotiation()) {
                    Some(found) => state.commit(self.finder.design, layout, id, found, &[], events),
                    None => {
                        if self.is_cut_off(connection, layout) {
                            cut_off.push(connection);
                        }
                        unrouted.push(id);
                    }
                }
            }
        }
        unrouted.sort_unstable();
        for pads in [&mut hemmed_in, &mut exits_taken] {
            pads.sort_unstable();
            pads.dedup();
        }
        Attempt {
            routed: state.routed,
            unrouted,
            hemmed_in,
            exits_taken,
            cut_off,
        }
    }

    /// Negotiated congestion in rounds, PathFinder's way: every connection takes
    /// a route each round, overlapping others at a price that rises from round
    /// to round, and history builds up where they meet; the connections that
    /// overlap are routed again in the next round. When no round brings fewer
    /// overlaps for ROUNDS_WITHOUT_GAIN rounds, the most overlapping routes are
    /// taken out until the rest is legal, and left in the queue, with those
    /// that found no way, for rip-up and reroute to finish: they may rip up
    /// each other, never the routes the rounds settled.
    fn settle_in_rounds(
        &mut self,
        state: &mut Negotiated,
        plan: &Plan,
        corridors: Option<&Corridors>,
        layout: &mut (impl LayoutAccess + Sync),
        cancel: &CancelToken,
        events: &impl Publisher<Event>,
    ) {
        let design = self.finder.design;
        let total = plan.connections.len();
        let no_rip_ups = vec![0_u32; total];
        let mut present = self.finder.profile.rip_up_cost;
        let mut failed = Vec::new();
        let (mut fewest_overlapping, mut rounds_without_gain) = (usize::MAX, 0);
        loop {
            while !state.queue.is_empty() && !cancel.is_cancelled() && !self.is_past_deadline() {
                let batch = self.next_batch(&mut state.queue, plan, corridors);
                let negotiation = search::Negotiation {
                    rip_ups: &no_rip_ups,
                    history: &state.history,
                    present: Some(present),
                };
                let found = self.find_all(&batch, plan, corridors, layout, Some(negotiation));
                for (id, found) in batch.iter().copied().zip(found) {
                    let found = match found {
                        Ok(found) => Some(found),
                        Err(NoRoute::Blocked { .. }) => None,
                        Err(NoRoute::NotFound) => {
                            let connection = plan.connections[id.index()];
                            self.find_wider(connection, layout, Some(negotiation))
                        }
                    };
                    match found {
                        Some(found) => {
                            layout.add_route(id, &found);
                            state.routed[id.index()] = Some(found);
                            let done = state.routed.iter().filter(|r| r.is_some()).count();
                            events.publish(Event::ConnectionRouted { done, total });
                        }
                        None => failed.push(id),
                    }
                }
            }
            if cancel.is_cancelled() || self.is_past_deadline() {
                break;
            }
            let overlapping: Vec<ConnectionId> = (0..total)
                .map(ConnectionId::from_index)
                .filter(|c| {
                    state.routed[c.index()]
                        .as_ref()
                        .is_some_and(|r| !routed_obstructions(layout, r).is_empty())
                })
                .collect();
            if overlapping.is_empty() {
                break;
            }
            if overlapping.len() < fewest_overlapping {
                (fewest_overlapping, rounds_without_gain) = (overlapping.len(), 0);
            } else {
                rounds_without_gain += 1;
                if rounds_without_gain >= ROUNDS_WITHOUT_GAIN {
                    break;
                }
            }
            for &id in &overlapping {
                if let Some(found) = &state.routed[id.index()] {
                    record_collisions(&mut state.history, design, layout, found);
                }
            }
            for &id in &overlapping {
                layout.remove_route(id);
                state.routed[id.index()] = None;
                state.queue.push_back(id);
            }
            present = present.saturating_mul(3) / 2;
        }
        // Legal at last: the routes that overlap most go, the rest stay.
        let mut left = failed;
        left.extend(state.queue.drain(..));
        loop {
            let worst = (0..total)
                .map(ConnectionId::from_index)
                .filter_map(|c| {
                    let overlaps =
                        routed_obstructions(layout, state.routed[c.index()].as_ref()?).len();
                    (overlaps > 0).then_some((overlaps, c))
                })
                .max();
            let Some((_, id)) = worst else { break };
            layout.remove_route(id);
            state.routed[id.index()] = None;
            left.push(id);
        }
        left.sort_unstable();
        left.dedup();
        // The routes the rounds settled stay: the leftovers rip up only each other.
        let max = self.finder.profile.max_rip_ups;
        for (rip_ups, routed) in state.rip_ups.iter_mut().zip(&state.routed) {
            if routed.is_some() {
                *rip_ups = max;
            }
        }
        state.most_routed = state.routed.iter().filter(|r| r.is_some()).count();
        state.queue.extend(left);
    }

    /// A search in windows grown WIDER_WINDOW times wider each, alone, until one
    /// finds a way or the last covered the whole board: the way round a row of
    /// headers or the board's own wiring is often longer than the margin.
    fn find_wider(
        &mut self,
        connection: Connection,
        layout: &impl LayoutAccess,
        negotiation: Option<search::Negotiation>,
    ) -> Option<Routes> {
        let board = self.finder.design.outline.bounds();
        let mut wider = self.finder.profile.clone();
        while !search::window(self.finder.design, wider.detour_margin, connection, None)
            .contains_rect(board)
        {
            wider.detour_margin = wider.detour_margin.saturating_mul(WIDER_WINDOW);
            let finder = Finder {
                profile: &wider,
                ..self.finder
            };
            if let Ok(found) = finder.search(
                &mut self.scratches[0],
                connection,
                layout,
                negotiation,
                None,
            ) {
                return Some(found);
            }
        }
        None
    }

    /// Whether the board's own copper, keepouts and edge leave no way at all
    /// between the pads of `connection`, whatever was routed.
    fn is_cut_off(&mut self, connection: Connection, layout: &impl LayoutAccess) -> bool {
        let board = self.finder.design.outline.bounds();
        let whole_board = Profile {
            detour_margin: (board.max.x - board.min.x).max(board.max.y - board.min.y),
            ..self.finder.profile.clone()
        };
        search::Search::new(
            self.finder.design,
            self.finder.rules,
            &whole_board,
            connection,
            layout,
            None,
            None,
        )
        .through_routed_copper()
        .run(&mut self.scratches[0])
        .is_err()
    }

    /// Takes up to the profile's `batch_size` connections from the front of
    /// `queue`, in order, whose territories overlap neither each other nor that
    /// of any connection they skip: so no connection of a batch can meet the
    /// copper of another, nor overtake one it could meet. Batches depend on the
    /// queue alone, never on the number of threads.
    fn next_batch(
        &self,
        queue: &mut VecDeque<ConnectionId>,
        plan: &Plan,
        corridors: Option<&Corridors>,
    ) -> Vec<ConnectionId> {
        let batch_size = self.finder.profile.batch_size as usize;
        // Bounds the cost of looking for room on boards with many connections.
        let scan_depth = batch_size * SCANNED_PER_BATCH_SLOT;
        let mut claimed: Vec<Rect> = Vec::new();
        let mut batch = Vec::new();
        let mut position = 0;
        while let Some(&id) = queue.get(position) {
            if batch.len() == batch_size || claimed.len() == scan_depth {
                break;
            }
            let territory = self
                .finder
                .territory(id, plan.connections[id.index()], corridors);
            let is_free = claimed.iter().all(|c| !c.intersects(territory));
            claimed.push(territory);
            if is_free {
                batch.extend(queue.remove(position));
            } else {
                position += 1;
            }
        }
        batch
    }

    /// The route each connection of `batch` finds in `layout`, in batch order.
    fn find_all(
        &mut self,
        batch: &[ConnectionId],
        plan: &Plan,
        corridors: Option<&Corridors>,
        layout: &(impl LayoutAccess + Sync),
        negotiation: Option<search::Negotiation>,
    ) -> Vec<Result<Routes, NoRoute>> {
        let finder = self.finder;
        let find = |scratch: &mut search::Scratch, id: ConnectionId| {
            let connection = plan.connections[id.index()];
            finder.find(scratch, id, connection, corridors, layout, negotiation)
        };
        let workers = self.scratches.len().min(batch.len());
        if workers <= 1 {
            let scratch = &mut self.scratches[0];
            return batch.iter().map(|&id| find(scratch, id)).collect();
        }
        // The largest search runs on the first scratch, which batches of one
        // connection have grown already, so the other scratches only grow as
        // large as the smaller searches they get: filling fresh memory for
        // every thread cost more than the threads saved.
        let mut order: Vec<usize> = (0..batch.len()).collect();
        order.sort_by_key(|&item| {
            let id = batch[item];
            Reverse(
                finder
                    .territory(id, plan.connections[id.index()], corridors)
                    .area(),
            )
        });
        let next = AtomicUsize::new(1);
        let work = |scratch: &mut search::Scratch, mut position: usize| {
            let mut done = Vec::new();
            while let Some(&item) = order.get(position) {
                done.push((item, find(scratch, batch[item])));
                position = next.fetch_add(1, Ordering::Relaxed);
            }
            done
        };
        let (first, others) = self.scratches[..workers]
            .split_first_mut()
            .unwrap_or_else(|| unreachable!("at least two workers"));
        let mut found: Vec<Result<Routes, NoRoute>> =
            batch.iter().map(|_| Err(NoRoute::NotFound)).collect();
        std::thread::scope(|scope| {
            let helpers: Vec<_> = others
                .iter_mut()
                .map(|scratch| {
                    let (next, work) = (&next, &work);
                    scope.spawn(move || work(scratch, next.fetch_add(1, Ordering::Relaxed)))
                })
                .collect();
            let mine = work(first, 0);
            // A helper only panics on a bug in the search; pass it on.
            let theirs = helpers.into_iter().flat_map(|helper| {
                helper
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            });
            for (item, routes) in mine.into_iter().chain(theirs) {
                found[item] = routes;
            }
        });
        found
    }
}

/// What a search needs besides its memory; shared by all threads.
#[derive(Clone, Copy)]
struct Finder<'a> {
    design: &'a Design,
    rules: &'a ConstraintSet,
    profile: &'a Profile,
}

impl Finder<'_> {
    /// Where the connection's copper may go, grown by the clearance other copper
    /// keeps from it: two connections with apart territories cannot meet.
    fn territory(
        &self,
        id: ConnectionId,
        connection: Connection,
        corridors: Option<&Corridors>,
    ) -> Rect {
        let anywhere = search::window(self.design, self.profile.detour_margin, connection, None);
        let window = match corridors.filter(|c| c.bounds(id).is_some()) {
            Some(corridors) => anywhere.union(search::window(
                self.design,
                self.profile.detour_margin,
                connection,
                Some((corridors, id)),
            )),
            None => anywhere,
        };
        let via_radius = self
            .rules
            .via_kinds(connection.net)
            .iter()
            .flat_map(|kind| &self.design.via_kinds[kind.index()].copper)
            .map(|copper| {
                let bounds = copper.shape.bounds();
                (bounds.max.x - bounds.min.x).max(bounds.max.y - bounds.min.y) / 2 + 1
            })
            .max()
            .unwrap_or(0);
        let copper_reach = via_radius.max(self.rules.track_width(connection.net) / 2 + 1);
        window.inflated(copper_reach + self.rules.largest_clearance())
    }

    /// A route inside the connection's corridor; when the corridor turns out to
    /// be blocked, as global routing only estimates room, anywhere in reach.
    fn find(
        &self,
        scratch: &mut search::Scratch,
        id: ConnectionId,
        connection: Connection,
        corridors: Option<&Corridors>,
        layout: &impl LayoutAccess,
        negotiation: Option<search::Negotiation>,
    ) -> Result<Routes, NoRoute> {
        let in_corridor = corridors.filter(|c| c.bounds(id).is_some()).and_then(|c| {
            self.search(scratch, connection, layout, negotiation, Some((c, id)))
                .ok()
        });
        in_corridor.map_or_else(
            || self.search(scratch, connection, layout, negotiation, None),
            Ok,
        )
    }

    fn search(
        &self,
        scratch: &mut search::Scratch,
        connection: Connection,
        layout: &impl LayoutAccess,
        negotiation: Option<search::Negotiation>,
        corridor: Option<(&Corridors, ConnectionId)>,
    ) -> Result<Routes, NoRoute> {
        let search = search::Search::new(
            self.design,
            self.rules,
            self.profile,
            connection,
            layout,
            negotiation,
            corridor,
        );
        let steps = search.run(scratch)?;
        Ok(path::to_routes(
            &steps,
            connection.net,
            self.rules.track_width(connection.net),
        ))
    }
}

/// What one way of negotiating routed, and why the rest is not.
struct Attempt {
    routed: Vec<Option<Routes>>,
    unrouted: Vec<ConnectionId>,
    hemmed_in: Vec<traxide_contracts::PadId>,
    exits_taken: Vec<traxide_contracts::PadId>,
    cut_off: Vec<Connection>,
}

impl Attempt {
    /// How far it got: nets completed first, as boards are judged by them,
    /// then connections routed.
    fn standing(&self, plan: &Plan) -> (usize, usize) {
        let nets: BTreeSet<NetId> = plan.connections.iter().map(|c| c.net).collect();
        let incomplete: BTreeSet<NetId> = (plan.connections.iter().zip(&self.routed))
            .filter(|(_, routed)| routed.is_none())
            .map(|(connection, _)| connection.net)
            .collect();
        let routed = self.routed.iter().filter(|r| r.is_some()).count();
        (nets.len() - incomplete.len(), routed)
    }

    fn take_out(&self, layout: &mut impl LayoutAccess) {
        for (i, routed) in self.routed.iter().enumerate() {
            if routed.is_some() {
                layout.remove_route(ConnectionId::from_index(i));
            }
        }
    }

    fn put_back(&self, layout: &mut impl LayoutAccess) {
        for (i, routed) in self.routed.iter().enumerate() {
            if let Some(routes) = routed {
                layout.add_route(ConnectionId::from_index(i), routes);
            }
        }
    }

    fn into_result(self, plan: &Plan) -> RoutingResult {
        let mut routes = Routes::default();
        for found in self.routed.into_iter().flatten() {
            routes.extend(found);
        }
        RoutingResult {
            routes,
            unrouted: (self.unrouted.into_iter())
                .map(|id| plan.connections[id.index()])
                .collect(),
            hemmed_in: self.hemmed_in,
            exits_taken: self.exits_taken,
            cut_off: self.cut_off,
        }
    }
}

/// Where routing stands: the routes so far and what negotiation has learnt.
struct Negotiated {
    routed: Vec<Option<Routes>>,
    rip_ups: Vec<u32>,
    queue: VecDeque<ConnectionId>,
    history: history::History,
    may_rip_up: bool,
    most_routed: usize,
    rip_ups_since_most: usize,
}

impl Negotiated {
    fn negotiation(&self) -> Option<search::Negotiation<'_>> {
        self.may_rip_up.then_some(search::Negotiation {
            rip_ups: &self.rip_ups,
            history: &self.history,
            present: None,
        })
    }

    /// Adds `found` for `id`, ripping up the routed connections it crosses,
    /// which never include another of `batch`.
    fn commit(
        &mut self,
        design: &Design,
        layout: &mut impl LayoutAccess,
        id: ConnectionId,
        found: Routes,
        batch: &[ConnectionId],
        events: &impl Publisher<Event>,
    ) {
        let displaced = routed_obstructions(layout, &found);
        if !displaced.is_empty() {
            record_collisions(&mut self.history, design, layout, &found);
        }
        debug_assert!(
            displaced.iter().all(|d| !batch.contains(d)),
            "connections of one batch keep out of each other's territory"
        );
        for displaced in displaced {
            if let Some(theirs) = &self.routed[displaced.index()] {
                events.publish(Event::RippedUp {
                    connection: displaced,
                    by: id,
                    at: where_they_meet(&found, theirs),
                });
            }
            layout.remove_route(displaced);
            self.routed[displaced.index()] = None;
            self.rip_ups[displaced.index()] += 1;
            self.rip_ups_since_most += 1;
            self.queue.push_back(displaced);
        }
        layout.add_route(id, &found);
        self.routed[id.index()] = Some(found);
        let total = self.routed.len();
        let done = self.routed.iter().filter(|r| r.is_some()).count();
        if done > self.most_routed {
            (self.most_routed, self.rip_ups_since_most) = (done, 0);
        } else if self.rip_ups_since_most >= STALLED_ROUNDS * total {
            self.may_rip_up = false;
        }
        events.publish(Event::ConnectionRouted { done, total });
    }
}

/// About where the copper of `ours` comes closest to that of `theirs`: the
/// middle between the nearest of each one's corners and the other's tracks.
fn where_they_meet(ours: &Routes, theirs: &Routes) -> Point {
    let pieces = |routes: &Routes| -> Vec<Segment> {
        let tracks = routes
            .tracks
            .iter()
            .flat_map(|t| t.path.windows(2).map(|pair| Segment::new(pair[0], pair[1])));
        let vias = routes
            .vias
            .iter()
            .map(|v| Segment::new(v.position, v.position));
        tracks.chain(vias).collect()
    };
    let (ours, theirs) = (pieces(ours), pieces(theirs));
    let nearest = |pieces: &[Segment], others: &[Segment]| {
        pieces
            .iter()
            .flat_map(|piece| [piece.a, piece.b])
            .flat_map(|corner| {
                others
                    .iter()
                    .map(move |other| (corner, other.closest_point(corner)))
            })
            .min_by_key(|&(corner, on_other)| corner.distance(on_other))
    };
    [nearest(&ours, &theirs), nearest(&theirs, &ours)]
        .into_iter()
        .flatten()
        .min_by_key(|&(corner, on_other)| corner.distance(on_other))
        .map_or(Point::default(), |(corner, on_other)| {
            Point::new(
                corner.x + (on_other.x - corner.x) / 2,
                corner.y + (on_other.y - corner.y) / 2,
            )
        })
}

/// Makes the places where `routes` overlap routed copper dearer for later searches.
fn record_collisions(
    history: &mut history::History,
    design: &Design,
    layout: &impl LayoutAccess,
    routes: &Routes,
) {
    for track in &routes.tracks {
        for pair in track.path.windows(2) {
            let segment = traxide_contracts::geometry::Segment::new(pair[0], pair[1]);
            if let Obstruction::Routed(_) =
                layout.track_obstruction(track.net, track.layer, segment, track.width)
            {
                history.record_track(track.layer, segment, track.width);
            }
        }
    }
    for via in &routes.vias {
        if let Obstruction::Routed(_) = layout.via_obstruction(via) {
            let copper = &design.via_kind(via.kind).copper;
            let radius = copper
                .iter()
                .map(|c| {
                    let bounds = c.shape.bounds();
                    (bounds.max.x - bounds.min.x) / 2
                })
                .max()
                .unwrap_or(0);
            history.record_via(via.position, radius, copper.iter().map(|c| c.layer));
        }
    }
}

/// The routed connections that `routes` would overlap, in id order.
fn routed_obstructions(layout: &impl LayoutAccess, routes: &Routes) -> Vec<ConnectionId> {
    let tracks = routes.tracks.iter().flat_map(|track| {
        track.path.windows(2).map(move |pair| {
            let segment = traxide_contracts::geometry::Segment::new(pair[0], pair[1]);
            layout.track_obstruction(track.net, track.layer, segment, track.width)
        })
    });
    let vias = routes.vias.iter().map(|via| layout.via_obstruction(via));
    let mut displaced: Vec<ConnectionId> = tracks
        .chain(vias)
        .flat_map(|obstruction| match obstruction {
            Obstruction::Routed(connections) => connections,
            // The search never crosses fixed obstructions, so none can appear here.
            Obstruction::None | Obstruction::Fixed => Vec::new(),
        })
        .collect();
    displaced.sort_unstable();
    displaced.dedup();
    displaced
}

#[cfg(test)]
mod tests {
    use super::*;
    use traxide_contracts::testing::{DesignBuilder, at};
    use traxide_contracts::{KeepoutScope, LayerId, PadId, Track, Via, ViaKindId};
    use traxide_events::CancelToken;
    use traxide_layout_access::Layout;
    use traxide_rules::Purpose;

    #[test]
    fn tidying_keeps_clear_of_a_route_that_runs_beyond_its_window() {
        // A runs over the wall's end at y 15 mm, far beyond its window, by two
        // vias its own window holds no way round. B passes under A's way down
        // at x 15 mm by two vias, which it could do without, were A not there.
        let wall = traxide_contracts::geometry::Shape::Polygon(
            traxide_contracts::geometry::Polygon::rectangle(Rect {
                min: at(9.5, 0.0),
                max: at(10.5, 14.0),
            }),
        );
        let design = DesignBuilder::new(30.0, 20.0)
            .keepout(wall, None, KeepoutScope::Everything)
            .smd("A1", Some("A"), at(5.0, 2.0), 0.6, 0)
            .smd("A2", Some("A"), at(15.0, 2.0), 0.6, 0)
            .smd("B1", Some("B"), at(13.0, 10.0), 0.6, 0)
            .smd("B2", Some("B"), at(18.0, 10.0), 0.6, 0)
            .build();
        let (a, b) = (design.net_named("A"), design.net_named("B"));
        let track = |net, layer, path: &[Point]| Track {
            net,
            layer: LayerId::from_index(layer),
            width: 200_000,
            path: path.to_vec(),
        };
        let via = |net, x, y| Via {
            net,
            kind: ViaKindId(0),
            position: at(x, y),
        };
        let over_the_wall = Routes {
            tracks: vec![
                track(a, 0, &[at(5.0, 2.0), at(5.0, 15.0)]),
                track(a, 1, &[at(5.0, 15.0), at(15.0, 15.0)]),
                track(a, 0, &[at(15.0, 15.0), at(15.0, 2.0)]),
            ],
            vias: vec![via(a, 5.0, 15.0), via(a, 15.0, 15.0)],
        };
        let under_a = Routes {
            tracks: vec![
                track(b, 0, &[at(13.0, 10.0), at(14.0, 10.0)]),
                track(b, 1, &[at(14.0, 10.0), at(16.0, 10.0)]),
                track(b, 0, &[at(16.0, 10.0), at(18.0, 10.0)]),
            ],
            vias: vec![via(b, 14.0, 10.0), via(b, 16.0, 10.0)],
        };
        let connection = |net, from: &str, to: &str| Connection {
            net,
            from: design.pad_named(from),
            to: design.pad_named(to),
        };
        let plan = Plan {
            connections: vec![connection(a, "A1", "A2"), connection(b, "B1", "B2")],
        };
        let profile = Profile {
            name: "side by side".into(),
            grid_pitch: 100_000,
            via_cost: 2_000_000,
            bend_cost: 100_000,
            detour_margin: 2_000_000,
            rip_up_cost: 5_000_000,
            max_rip_ups: 10,
            history_cost: 0,
            heuristic_weight: 1.0,
            global_routing: false,
            batch_size: 16,
        };
        let rules = traxide_rules::compile(
            &design,
            Purpose::Routing {
                grid_pitch: 100_000,
            },
        );
        let mut layout = Layout::build(&design, &rules);
        layout.add_route(ConnectionId::from_index(0), &over_the_wall);
        layout.add_route(ConnectionId::from_index(1), &under_a);
        let mut attempt = Attempt {
            routed: vec![Some(over_the_wall), Some(under_a)],
            ..attempt(&[])
        };
        Router::new(&design, &rules, &profile).tidy(
            &mut attempt,
            &plan,
            &mut layout,
            &CancelToken::new(),
        );
        let tidied = attempt.into_result(&plan).routes;
        let rules = traxide_rules::compile(&design, Purpose::Verification);
        let verdict = traxide_verification::verify(&design, &rules, &tidied);
        let broken: Vec<_> = (verdict.violations.iter())
            .filter(|v| !matches!(v, traxide_contracts::Violation::Unconnected { .. }))
            .collect();
        assert!(broken.is_empty(), "{broken:?}");
    }

    fn attempt(routed: &[bool]) -> Attempt {
        Attempt {
            routed: routed.iter().map(|&r| r.then(Routes::default)).collect(),
            unrouted: (0..routed.len())
                .filter(|&i| !routed[i])
                .map(ConnectionId::from_index)
                .collect(),
            hemmed_in: Vec::new(),
            exits_taken: Vec::new(),
            cut_off: Vec::new(),
        }
    }

    #[test]
    fn an_attempt_that_completes_more_nets_stands_higher_than_one_that_routes_more_connections() {
        let connection = |net| Connection {
            net: NetId::from_index(net),
            from: PadId::from_index(0),
            to: PadId::from_index(1),
        };
        // Net 0 has four connections, net 1 one.
        let plan = Plan {
            connections: vec![
                connection(0),
                connection(0),
                connection(0),
                connection(0),
                connection(1),
            ],
        };
        let net_1_complete = attempt(&[true, false, false, false, true]);
        let more_connections = attempt(&[true, true, true, false, false]);
        assert!(net_1_complete.standing(&plan) > more_connections.standing(&plan));
        assert!(
            more_connections.standing(&plan)
                > attempt(&[true, false, false, false, false]).standing(&plan)
        );
    }
}
