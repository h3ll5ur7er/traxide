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
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use traxide_contracts::geometry::{Point, Rect, Segment};
use traxide_contracts::{
    Connection, ConnectionId, ConstraintSet, Corridors, Design, Event, LayoutAccess, NetId,
    Obstruction, PadId, Plan, Profile, Routes, RoutingResult,
};
use traxide_events::{CancelToken, Publisher, Silent};

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

/// How many times dearer a via is when routing is tidied, hops are taken out
/// and trees grow: negotiating at that price costs connections and time, but
/// once everything is routed a connection may well find a way with fewer
/// vias among the rest (TRX-72). Since what a hop's way crosses finds room
/// further off, dearer pays: with `--profile fast`, TokenBoard and ten corpus
/// boards together 1,293 vias at 4 times, 1,221 at 6, 1,201 at 8, 1,162 at 10
/// (TRX-88).
const TIDYING_VIA_COST: i64 = 10;

/// How boldly a search heads for its target when routing is tidied: any way
/// with fewer vias will do, and the profile's weight, searching for the
/// cheapest, made tidying take as long as routing on small boards. 1.5
/// instead of 2 with vias 10 times dearer: 1,090 vias instead of 1,162, in
/// 17 % more time; with vias 12 times dearer 1,120 (TRX-88).
const TIDYING_HEURISTIC_WEIGHT: f64 = 1.5;

/// Taking out hops, crossing a routed connection costs this many eighths of
/// a via, however wide the two tracks and the clearance between them. Priced
/// per grid edge overlapped instead, at an eighth of a via, crossing
/// TokenBoard's 0.2 mm tracks cost about a via but CNH's wider ones over two,
/// so no wide track moved for a hop: CNH kept 52 vias, 19 now. With
/// `--profile fast`, TokenBoard and 28 corpus boards 1,618 vias instead of
/// 1,686, in 53 % more time; at half a via 1,640, five eighths 1,650
/// (TRX-87, TRX-92).
const CROSSING_EIGHTHS_OF_A_VIA: i64 = 3;

/// The rounds after the first: their prices, as factors of the profile's
/// price of a via and of running across a layer's direction, and whether
/// they route the board as is too, which only the price of a via moves.
/// Vias twice as dear throughout routed TokenBoard with 275 vias instead of
/// 314, 8088sbc with 73 instead of 93, CNH with 66 instead of 84, but
/// LogicBoard with 18 nets fewer within 90 s (TRX-80); layer directions half
/// as dear again or a third cheaper, Sofle with 364 or 385 vias instead of
/// 386, CNH with 45 or 63 instead of 64, TokenBoard with 291 instead of 267
/// (TRX-84): which prices suit a board only routing tells.
const LATER_ROUNDS: [(i64, f64, bool); 5] = [
    (2, 1.0, true),
    (1, 1.5, false),
    (2, 1.5, false),
    (1, 0.7, false),
    (2, 0.7, false),
];

/// How many times further than the profile's detour margin a connection that
/// a hop's new way crosses may stray to find room again: in its own window,
/// 42 of 156 attempts on TokenBoard failed. Measured with `--profile fast`
/// on TokenBoard and ten corpus boards together: 1,592 vias in its own
/// window, 1,509 three times as far, 1,492 six times, 1,548 widening four
/// times at a time only where none was found, for 6 % more time (TRX-85).
const CROSSED_ROOM: i32 = 6;

/// Moving tracks makes room for further hops to go: on TokenBoard a second
/// pass took out 16 more vias in 10 s, a third 2 more in 6 s (TRX-76). With
/// crossings an eighth of a via per edge, a third pass leaves TokenBoard and
/// the ten boards with 1,311 vias instead of 1,355, in 14 % more time; a
/// fourth TokenBoard with 318 instead of 332 and the ten with 975 instead of
/// 979, in 8 % more; a fifth takes out none on TokenBoard (TRX-87).
const MOST_HOP_PASSES: usize = 4;

/// Past its deadline, routing searches for what is still queued for this
/// share of the time it was given more, a second at least, and leaves the
/// rest unrouted: LogicBoard's searches without rip-up took up to 43 s past a
/// 90 s limit, as each way of running the layers drained its queue on one
/// thread.
const SEARCHING_PAST_THE_DEADLINE: u32 = 10;
const LEAST_SEARCHING_PAST_THE_DEADLINE: Duration = Duration::from_secs(1);

pub struct Router<'a> {
    finder: Finder<'a>,
    /// One per thread: each search needs its own memory.
    scratches: Vec<search::Scratch>,
    /// When negotiation stops ripping up, whatever it would still gain.
    deadline: Option<Instant>,
    /// When it stops searching (SEARCHING_PAST_THE_DEADLINE).
    searching_ends: Option<Instant>,
    /// Whether the board's own copper cuts each connection asked about off,
    /// which no routing changes: a search for one cut off floods all it can
    /// reach, again on every attempt.
    cut_off: HashMap<Connection, bool>,
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
                directions: search::Directions::default(),
            },
            scratches: vec![search::Scratch::default()],
            deadline: None,
            searching_ends: None,
            cut_off: HashMap::new(),
        }
    }

    /// Searches the connections of a batch on up to `threads` threads. The
    /// routes are the same on any number of them.
    /// Stops negotiation at `deadline`: what is still queued then routes once
    /// more without crossing anything, each in its own window only, so routing
    /// ends a little later: a tenth of the time given at most, or a second
    /// (SEARCHING_PAST_THE_DEADLINE).
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        let given = deadline.saturating_duration_since(Instant::now());
        let past = (given / SEARCHING_PAST_THE_DEADLINE).max(LEAST_SEARCHING_PAST_THE_DEADLINE);
        self.deadline = Some(deadline);
        self.searching_ends = Some(deadline + past);
        self
    }

    fn is_past_deadline(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    fn is_done_searching(&self) -> bool {
        self.searching_ends
            .is_some_and(|ends| Instant::now() >= ends)
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
    ///
    /// The board is routed so three ways side by side (`route_side_by_side`):
    /// as is, and with each layer's tracks running one way, crosswise to the
    /// next, in either orientation (`ways_to_run_layers`). Where time is left,
    /// it is routed again in the profile's further rounds, at the prices of
    /// LATER_ROUNDS. The routing that completes more, or as much with fewer
    /// vias, stays, the first of equals: running layers crosswise leaves fewer
    /// hops to take out where it suits the board, and only routing tells which
    /// way, and which prices, do (TRX-77, TRX-79, TRX-80, TRX-84).
    pub fn route_plan<L: LayoutAccess + Clone + Send + Sync>(
        &mut self,
        plan: &Plan,
        corridors: Option<&Corridors>,
        layout: &mut L,
        cancel: &CancelToken,
        events: &impl Publisher<Event>,
    ) -> RoutingResult {
        let ways = self.ways_to_run_layers();
        if ways.is_empty() {
            return self
                .route_attempt(plan, corridors, layout, cancel, events)
                .into_result(plan);
        }
        let threads = self.scratches.len();
        let unrouted = layout.clone();
        let all_ways: Vec<search::Directions> = std::iter::once(search::Directions::default())
            .chain(ways)
            .collect();
        let rounds = (self.finder.profile.rounds as usize).clamp(1, LATER_ROUNDS.len() + 1);
        let of = rounds as u32;
        events.publish(Event::RoundStarted { round: 1, of });
        let mut best = self.route_side_by_side(&all_ways, plan, corridors, layout, cancel, events);
        let later = LATER_ROUNDS.iter().take(rounds - 1);
        for (round, &(dearer_vias, layers, as_is)) in (2..).zip(later) {
            if cancel.is_cancelled() || self.is_past_deadline() {
                break;
            }
            events.publish(Event::RoundStarted { round, of });
            let priced = Profile {
                via_cost: self.finder.profile.via_cost * dearer_vias,
                against_layer_direction: self.finder.profile.against_layer_direction * layers,
                ..self.finder.profile.clone()
            };
            let mut router = self.alike(&priced, search::Directions::default(), threads);
            let ways: Vec<search::Directions> = (as_is.then(search::Directions::default))
                .into_iter()
                .chain(router.ways_to_run_layers())
                .collect();
            let mut routed = unrouted.clone();
            let attempt =
                router.route_side_by_side(&ways, plan, corridors, &mut routed, cancel, events);
            if attempt.is_better_than(&best, plan) {
                best = attempt;
                *layout = routed;
            }
        }
        best.into_result(plan)
    }

    /// Routes the board with its layers running each of `ways`, side by side,
    /// each in a layout of its own and on its share of the threads, the first
    /// here, publishing progress; keeps the best routing in `layout`.
    fn route_side_by_side<L: LayoutAccess + Clone + Send + Sync>(
        &mut self,
        ways: &[search::Directions],
        plan: &Plan,
        corridors: Option<&Corridors>,
        layout: &mut L,
        cancel: &CancelToken,
        events: &impl Publisher<Event>,
    ) -> Attempt {
        let Some((&here, elsewhere)) = ways.split_first() else {
            return self.route_attempt(plan, corridors, layout, cancel, events);
        };
        // The first routing takes the threads left over: on 4 threads, three
        // routings would leave one idle.
        let threads = (self.scratches.len() / ways.len()).max(1);
        let others: Vec<(Router<'_>, L)> = (elsewhere.iter())
            .map(|&directions| {
                let router = self.alike(self.finder.profile, directions, threads);
                (router, layout.clone())
            })
            .collect();
        let left_over = self
            .scratches
            .len()
            .saturating_sub(threads * elsewhere.len());
        self.scratches.truncate(left_over.max(1));
        let directions = std::mem::replace(&mut self.finder.directions, here);
        let (first, others) = std::thread::scope(|scope| {
            let running: Vec<_> = (others.into_iter())
                .map(|(mut router, mut layout)| {
                    scope.spawn(move || {
                        let attempt =
                            router.route_attempt(plan, corridors, &mut layout, cancel, &Silent);
                        (attempt, layout)
                    })
                })
                .collect();
            let first = self.route_attempt(plan, corridors, layout, cancel, events);
            let others: Vec<(Attempt, L)> = (running.into_iter())
                .map(|routing| routing.join().expect("a routing of the board panicked"))
                .collect();
            (first, others)
        });
        self.finder.directions = directions;
        let mut best = first;
        for (attempt, routed_layout) in others {
            if attempt.is_better_than(&best, plan) {
                best = attempt;
                *layout = routed_layout;
            }
        }
        best
    }

    /// The ways each layer's tracks may run besides any way, crosswise to the
    /// next routable layer: none where the profile asks no direction, or the
    /// board has fewer than two layers to route on.
    fn ways_to_run_layers(&self) -> Vec<search::Directions> {
        let share = self.finder.profile.against_layer_direction;
        let routable: Vec<usize> = (self.finder.design.layers.iter().enumerate())
            .filter(|(_, layer)| layer.routable)
            .map(|(i, _)| i)
            .collect();
        if share <= 0.0 || routable.len() < 2 || routable.iter().any(|&i| i >= 64) {
            return Vec::new();
        }
        [0, 1]
            .map(|first| search::Directions {
                against_per_mille: (share * 1000.0).round() as i64,
                horizontal_layers: (routable.iter().enumerate())
                    .filter(|(n, _)| (n + first).is_multiple_of(2))
                    .map(|(_, &layer)| 1_u64 << layer)
                    .sum(),
            })
            .to_vec()
    }

    /// A router like this one, routing by `profile`, its layers running the
    /// way `directions` says, searching on `threads` threads.
    fn alike<'b>(
        &self,
        profile: &'b Profile,
        directions: search::Directions,
        threads: usize,
    ) -> Router<'b>
    where
        'a: 'b,
    {
        Router {
            finder: Finder {
                design: self.finder.design,
                rules: self.finder.rules,
                profile,
                directions,
            },
            scratches: (0..threads).map(|_| search::Scratch::default()).collect(),
            deadline: self.deadline,
            searching_ends: self.searching_ends,
            cut_off: self.cut_off.clone(),
        }
    }

    /// Negotiates the board, a second time settled in rounds where that
    /// leaves connections unrouted, and tidies what the better of the two
    /// routed.
    fn route_attempt(
        &mut self,
        plan: &Plan,
        corridors: Option<&Corridors>,
        layout: &mut (impl LayoutAccess + Sync),
        cancel: &CancelToken,
        events: &impl Publisher<Event>,
    ) -> Attempt {
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
            if !best.unrouted.is_empty() {
                self.shove_aside_for_unrouted(&mut best, plan, layout, cancel);
            }
            self.tidy(&mut best, plan, layout, cancel);
            let mut changed = None;
            for _ in 0..MOST_HOP_PASSES {
                let moved = self.take_out_hops(&mut best, plan, layout, changed.as_deref(), cancel);
                if moved.is_empty() {
                    break;
                }
                changed = Some(moved);
            }
            self.continue_lanes(&mut best, plan, layout, cancel);
            self.grow_trees(&mut best, plan, layout, cancel);
        }
        best
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

    /// Routes each connection that negotiation left unrouted once more,
    /// crossing routed connections at a price for each crossing, and moves
    /// what its way crosses aside (`move_aside`): it routes then if all of
    /// them still route. Negotiation rips up what is in a connection's way
    /// and leaves it unrouted until it routes again, and gives up after so
    /// many rip-ups; this moves nothing unless it routes elsewhere (TRX-37).
    fn shove_aside_for_unrouted(
        &mut self,
        attempt: &mut Attempt,
        plan: &Plan,
        layout: &mut (impl LayoutAccess + Sync),
        cancel: &CancelToken,
    ) {
        let (dearer, roomier) = self.moving_profiles();
        let finder = Finder {
            profile: &dearer,
            ..self.finder
        };
        let room_finder = Finder {
            profile: &roomier,
            ..self.finder
        };
        let no_history = history::History::new(self.finder.design, 0);
        let no_rip_ups = vec![0; attempt.routed.len()];
        let nets: Vec<NetId> = plan.connections.iter().map(|c| c.net).collect();
        let crossing = crossing_moved_aside(&dearer, &no_rip_ups, &no_history, &nets);
        let deadline = self.deadline;
        let scratch = &mut self.scratches[0];
        let mut still_unrouted = Vec::new();
        for id in std::mem::take(&mut attempt.unrouted) {
            if cancel.is_cancelled() || deadline.is_some_and(|d| Instant::now() >= d) {
                still_unrouted.push(id);
                continue;
            }
            let connection = plan.connections[id.index()];
            let Ok(found) = finder.search(scratch, connection, layout, Some(crossing), None) else {
                still_unrouted.push(id);
                continue;
            };
            let Some(moved) = move_aside(
                &room_finder,
                scratch,
                plan,
                &attempt.routed,
                layout,
                id,
                &found,
            ) else {
                still_unrouted.push(id);
                continue;
            };
            for (other, route) in moved {
                attempt.routed[other.index()] = Some(route);
            }
            attempt.routed[id.index()] = Some(found);
        }
        attempt.unrouted = still_unrouted;
    }

    /// The profiles of the stages that move routed copper: vias
    /// TIDYING_VIA_COST times dearer, and as such, searching CROSSED_ROOM
    /// times as far beyond the pads, for what moves aside.
    fn moving_profiles(&self) -> (Profile, Profile) {
        let dearer = Profile {
            via_cost: self.finder.profile.via_cost * TIDYING_VIA_COST,
            heuristic_weight: TIDYING_HEURISTIC_WEIGHT,
            ..self.finder.profile.clone()
        };
        let roomier = Profile {
            detour_margin: dearer.detour_margin.saturating_mul(CROSSED_ROOM),
            ..dearer.clone()
        };
        (dearer, roomier)
    }

    /// Takes out the vias of each routed connection that changes layers more
    /// often than it must, where moving the copper it passes makes room.
    /// Tidying searched it among the rest crossing nothing; now it may cross
    /// routed connections at a price for each crossing
    /// (CROSSING_EIGHTHS_OF_A_VIA), with vias as dear as tidying makes them,
    /// and takes the way found if it has fewer vias. What it crosses moves
    /// aside (`move_aside`). The change stays only if all of them route and
    /// their vias together fall: a hop
    /// under a track that could run elsewhere goes, a hop under a track that
    /// must stay where it is stays. Where `near` is given, only connections
    /// whose window meets one of its areas are tried: elsewhere nothing
    /// changed since they were tried last. Gives the areas of the copper that
    /// moved.
    fn take_out_hops(
        &mut self,
        attempt: &mut Attempt,
        plan: &Plan,
        layout: &mut (impl LayoutAccess + Sync),
        near: Option<&[Rect]>,
        cancel: &CancelToken,
    ) -> Vec<Rect> {
        let (dearer, roomier) = self.moving_profiles();
        let finder = Finder {
            profile: &dearer,
            ..self.finder
        };
        let room_finder = Finder {
            profile: &roomier,
            ..self.finder
        };
        let no_history = history::History::new(self.finder.design, 0);
        let no_rip_ups = vec![0; attempt.routed.len()];
        let nets: Vec<NetId> = plan.connections.iter().map(|c| c.net).collect();
        let crossing = crossing_moved_aside(&dearer, &no_rip_ups, &no_history, &nets);
        let fewest_of: Vec<usize> = plan
            .connections
            .iter()
            .map(|&connection| self.fewest_vias(connection))
            .collect();
        let deadline = self.deadline;
        let scratch = &mut self.scratches[0];
        let design = self.finder.design;
        let margin = self.finder.profile.detour_margin;
        let mut moved_areas = Vec::new();
        for (i, &fewest) in fewest_of.iter().enumerate() {
            if cancel.is_cancelled() || deadline.is_some_and(|d| Instant::now() >= d) {
                return Vec::new();
            }
            let connection = plan.connections[i];
            let Some(routes) = attempt.routed[i].clone() else {
                continue;
            };
            if routes.vias.len() <= fewest {
                continue;
            }
            if let Some(near) = near {
                let window = search::window(design, margin, connection, None);
                if !near.iter().any(|area| area.intersects(window)) {
                    continue;
                }
            }
            let id = ConnectionId::from_index(i);
            layout.remove_route(id);
            let moved = finder
                .search(scratch, connection, layout, Some(crossing), None)
                .ok()
                .filter(|found| found.vias.len() < routes.vias.len());
            let Some(moved) = moved else {
                layout.add_route(id, &routes);
                continue;
            };
            let Some(rerouted) = move_aside(
                &room_finder,
                scratch,
                plan,
                &attempt.routed,
                layout,
                id,
                &moved,
            ) else {
                layout.add_route(id, &routes);
                continue;
            };
            let vias_of = |routed: &Option<Routes>| routed.as_ref().map_or(0, |r| r.vias.len());
            let before = routes.vias.len()
                + (rerouted.iter())
                    .map(|(other, _)| vias_of(&attempt.routed[other.index()]))
                    .sum::<usize>();
            let after = moved.vias.len()
                + rerouted
                    .iter()
                    .map(|(_, route)| route.vias.len())
                    .sum::<usize>();
            if after < before {
                let old = std::iter::once(&routes).chain(
                    (rerouted.iter())
                        .filter_map(|(other, _)| attempt.routed[other.index()].as_ref()),
                );
                let new = std::iter::once(&moved).chain(rerouted.iter().map(|(_, route)| route));
                moved_areas.extend(old.chain(new).filter_map(copper_bounds));
                attempt.routed[i] = Some(moved);
                for (other, route) in rerouted {
                    attempt.routed[other.index()] = Some(route);
                }
            } else {
                layout.remove_route(id);
                layout.add_route(id, &routes);
                for (other, _) in &rerouted {
                    layout.remove_route(*other);
                    if let Some(theirs) = &attempt.routed[other.index()] {
                        layout.add_route(*other, theirs);
                    }
                }
            }
        }
        moved_areas
    }

    /// Routes each routed connection once more from any copper of its net
    /// already joined to its first pad to any joined to its second, among the
    /// rest without crossing them, with vias as dear as tidying makes them,
    /// and keeps the new way if it has fewer vias, or as many and is shorter:
    /// a pad beside its net's track joins the track, as a tree grows, rather
    /// than running beside it to a pad further on. A connection that another
    /// now ends on stays where it is from then on, so nothing comes apart, and
    /// so does one that goes on from the free end of a lane of the board's
    /// wiring, which would otherwise be left dangling;
    /// so this comes last, when no rip-up moves anything any more. The
    /// shortest routes go first, so branches join trunks rather than trunks
    /// moving onto their branches.
    fn grow_trees(
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
        let deadline = self.deadline;
        let scratch = &mut self.scratches[0];
        let mut carrying = vec![false; attempt.routed.len()];
        let mut order: Vec<usize> = (0..attempt.routed.len()).collect();
        order.sort_by_key(|&i| (attempt.routed[i].as_ref().map_or(0, track_length), i));
        for i in order {
            if cancel.is_cancelled() || deadline.is_some_and(|d| Instant::now() >= d) {
                return;
            }
            let connection = plan.connections[i];
            let Some(routes) = attempt.routed[i].clone() else {
                continue;
            };
            if carrying[i] || continues_a_lane(self.finder.design, connection, &routes) {
                continue;
            }
            let sides = joined_without(plan, &attempt.routed, i);
            let copper_of = |side: &[usize]| -> Vec<Routes> {
                side.iter()
                    .filter_map(|&other| attempt.routed[other].clone())
                    .collect()
            };
            let (from, to) = (copper_of(&sides[0]), copper_of(&sides[1]));
            if from.is_empty() && to.is_empty() {
                continue;
            }
            let id = ConnectionId::from_index(i);
            layout.remove_route(id);
            // A way of no copper starts where the copper of both sides lies on
            // top of each other, which pulling tight may part, as it keeps
            // only the points where a track ends on another.
            let grown = finder
                .search_joining(scratch, connection, layout, &from, &to)
                .ok()
                .filter(|grown| !grown.tracks.is_empty() || !grown.vias.is_empty())
                .filter(|grown| is_tidier(grown, &routes));
            let Some(grown) = grown else {
                layout.add_route(id, &routes);
                continue;
            };
            let design = self.finder.design;
            let Some(holding) =
                holding_pads_left(design, connection, &grown, &sides, &attempt.routed)
            else {
                layout.add_route(id, &routes);
                continue;
            };
            for other in holding {
                carrying[other] = true;
            }
            let pads = [connection.from, connection.to].map(|pad| design.pad(pad).position);
            for &other in sides.iter().flatten() {
                if let Some(theirs) = &attempt.routed[other]
                    && ends_on(&grown, theirs, pads)
                {
                    carrying[other] = true;
                }
            }
            layout.add_route(id, &grown);
            attempt.routed[i] = Some(grown);
        }
    }

    /// Routes each routed connection once more whose pad has lanes of the
    /// board's wiring that no route of its net continues, reaching the pad at
    /// their free ends only, among the rest without crossing them: such a
    /// lane is left a dangling stub where a route joins its pad elsewhere,
    /// being shorter. Where no such way is found, the route stays.
    fn continue_lanes(
        &mut self,
        attempt: &mut Attempt,
        plan: &Plan,
        layout: &mut (impl LayoutAccess + Sync),
        cancel: &CancelToken,
    ) {
        let design = self.finder.design;
        for i in 0..attempt.routed.len() {
            if cancel.is_cancelled() || self.is_past_deadline() {
                return;
            }
            let connection = plan.connections[i];
            if attempt.routed[i].is_none() {
                continue;
            }
            let continued = |at: Point| {
                (plan.connections.iter().zip(&attempt.routed))
                    .filter(|(other, _)| other.net == connection.net)
                    .filter_map(|(_, routed)| routed.as_ref())
                    .any(|routes| reaches(routes, at))
            };
            let leaves_a_lane_dangling = [connection.from, connection.to].iter().any(|&pad| {
                let ends = lanes::free_ends(design, design.pad(pad));
                !ends.is_empty() && !ends.iter().any(|end| continued(end.at))
            });
            if !leaves_a_lane_dangling {
                continue;
            }
            let id = ConnectionId::from_index(i);
            layout.remove_route(id);
            if let Ok(by_lanes) =
                self.finder
                    .search_by_lanes(&mut self.scratches[0], connection, layout)
            {
                attempt.routed[i] = Some(by_lanes);
            }
            if let Some(routes) = &attempt.routed[i] {
                layout.add_route(id, routes);
            }
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
            if cancel.is_cancelled() || self.is_done_searching() {
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
                if self.is_cut_off(connection, layout) {
                    cut_off.push(connection);
                    unrouted.push(id);
                    continue;
                }
                match self.find_wider(connection, layout, state.negotiation()) {
                    Some(found) => state.commit(self.finder.design, layout, id, found, &[], events),
                    None => unrouted.push(id),
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
                    present: Some(search::Present::PerEdge(present)),
                };
                let found = self.find_all(&batch, plan, corridors, layout, Some(negotiation));
                for (id, found) in batch.iter().copied().zip(found) {
                    let found = match found {
                        Ok(found) => Some(found),
                        Err(NoRoute::Blocked { .. }) => None,
                        Err(NoRoute::NotFound) => {
                            let connection = plan.connections[id.index()];
                            if self.is_cut_off(connection, layout) {
                                None
                            } else {
                                self.find_wider(connection, layout, Some(negotiation))
                            }
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
    /// between the pads of `connection`, whatever was routed; searched once.
    fn is_cut_off(&mut self, connection: Connection, layout: &impl LayoutAccess) -> bool {
        if let Some(&known) = self.cut_off.get(&connection) {
            return known;
        }
        let board = self.finder.design.outline.bounds();
        let whole_board = Profile {
            detour_margin: (board.max.x - board.min.x).max(board.max.y - board.min.y),
            ..self.finder.profile.clone()
        };
        let cut_off = search::Search::new(
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
        .is_err();
        self.cut_off.insert(connection, cut_off);
        cut_off
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
        let (finder, known) = (self.finder, &self.cut_off);
        let find = |scratch: &mut search::Scratch, id: ConnectionId| {
            let connection = plan.connections[id.index()];
            if known.get(&connection) == Some(&true) {
                return Err(NoRoute::NotFound);
            }
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
    directions: search::Directions,
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
        )
        .with_directions(self.directions);
        let steps = search.run(scratch)?;
        Ok(self.routes(&steps, connection))
    }

    /// A route reaching the pads with lanes of the board's wiring only at the
    /// lanes' free ends, crossing nothing routed.
    /// A route from any of `from`, copper already joined to the first pad,
    /// or the pad itself, to any of `to` or the second pad, crossing nothing.
    fn search_joining(
        &self,
        scratch: &mut search::Scratch,
        connection: Connection,
        layout: &impl LayoutAccess,
        from: &[Routes],
        to: &[Routes],
    ) -> Result<Routes, NoRoute> {
        let search = search::Search::new(
            self.design,
            self.rules,
            self.profile,
            connection,
            layout,
            None,
            None,
        )
        .with_directions(self.directions);
        let steps = search.joining(from, to).run(scratch)?;
        Ok(self.routes(&steps, connection))
    }

    fn search_by_lanes(
        &self,
        scratch: &mut search::Scratch,
        connection: Connection,
        layout: &impl LayoutAccess,
    ) -> Result<Routes, NoRoute> {
        let search = search::Search::new(
            self.design,
            self.rules,
            self.profile,
            connection,
            layout,
            None,
            None,
        )
        .with_directions(self.directions);
        let steps = search.by_lanes_only().run(scratch)?;
        Ok(self.routes(&steps, connection))
    }

    fn routes(&self, steps: &[path::Step], connection: Connection) -> Routes {
        path::to_routes(
            steps,
            connection.net,
            self.rules.track_width(connection.net),
        )
    }
}

/// Crossing routed connections at CROSSING_EIGHTHS_OF_A_VIA of a via of
/// `profile` each, whatever their rip-ups and history, as what is crossed
/// moves aside then.
fn crossing_moved_aside<'a>(
    profile: &Profile,
    rip_ups: &'a [u32],
    history: &'a history::History,
    nets: &'a [NetId],
) -> search::Negotiation<'a> {
    search::Negotiation {
        rip_ups,
        history,
        present: Some(search::Present::PerCrossing {
            price: profile.via_cost * CROSSING_EIGHTHS_OF_A_VIA / 8,
            nets,
        }),
    }
}

/// Puts `found`, a way for connection `id` that may cross routed connections,
/// into `layout`, where `id` has no route, and routes each connection it
/// crosses once more among the rest without crossing them: the new routes of
/// those, `layout` holding them; or `None` where one finds no way, `layout`
/// as it was.
fn move_aside(
    room_finder: &Finder,
    scratch: &mut search::Scratch,
    plan: &Plan,
    routed: &[Option<Routes>],
    layout: &mut impl LayoutAccess,
    id: ConnectionId,
    found: &Routes,
) -> Option<Vec<(ConnectionId, Routes)>> {
    let crossed = routed_obstructions(layout, found);
    for &other in &crossed {
        layout.remove_route(other);
    }
    layout.add_route(id, found);
    let mut moved = Vec::new();
    for &other in &crossed {
        let connection = plan.connections[other.index()];
        match room_finder.search(scratch, connection, layout, None, None) {
            Ok(route) => {
                layout.add_route(other, &route);
                moved.push((other, route));
            }
            Err(_) => {
                for (done, _) in &moved {
                    layout.remove_route(*done);
                }
                layout.remove_route(id);
                for &other in &crossed {
                    if let Some(theirs) = &routed[other.index()] {
                        layout.add_route(other, theirs);
                    }
                }
                return None;
            }
        }
    }
    Some(moved)
}

/// The connections of connection `i`'s net joined, by routed connections
/// other than `i`, to its first pad and to its second.
fn joined_without(plan: &Plan, routed: &[Option<Routes>], i: usize) -> [Vec<usize>; 2] {
    let net = plan.connections[i].net;
    let mut parent: HashMap<PadId, PadId> = HashMap::new();
    let members: Vec<usize> = (0..plan.connections.len())
        .filter(|&k| k != i && plan.connections[k].net == net && routed[k].is_some())
        .collect();
    for &k in &members {
        let c = plan.connections[k];
        let (a, b) = (root(&mut parent, c.from), root(&mut parent, c.to));
        if a != b {
            parent.insert(a, b);
        }
    }
    let ends = [plan.connections[i].from, plan.connections[i].to];
    ends.map(|end| {
        let side = root(&mut parent, end);
        members
            .iter()
            .copied()
            .filter(|&k| root(&mut parent, plan.connections[k].from) == side)
            .collect()
    })
}

/// The pad standing for `pad`'s group of joined pads.
fn root(parent: &mut HashMap<PadId, PadId>, pad: PadId) -> PadId {
    let up = *parent.get(&pad).unwrap_or(&pad);
    if up == pad {
        return pad;
    }
    let top = root(parent, up);
    parent.insert(pad, top);
    top
}

fn track_length(routes: &Routes) -> i64 {
    (routes.tracks.iter())
        .flat_map(|t| t.path.windows(2))
        .map(|w| w[0].distance(w[1]))
        .sum()
}

/// Whether `routes` go on from the free end of the board's wiring that a pad
/// of `connection` starts: moved, they would leave that end dangling.
fn continues_a_lane(design: &Design, connection: Connection, routes: &Routes) -> bool {
    [connection.from, connection.to].into_iter().any(|pad| {
        lanes::free_ends(design, design.pad(pad))
            .iter()
            .any(|end| reaches(routes, end.at))
    })
}

/// Whether `new` has fewer vias than `old`, or as many and shorter tracks.
fn is_tidier(new: &Routes, old: &Routes) -> bool {
    (new.vias.len(), track_length(new)) < (old.vias.len(), track_length(old))
}

/// Whether a track of `routes` starts or ends on `theirs`, elsewhere than at
/// one of its own `pads`, which joins it to what ends there without them; or
/// a via of `routes` stands on their copper, as a route of a via alone joins
/// copper crossing on two layers.
fn ends_on(routes: &Routes, theirs: &Routes, pads: [Point; 2]) -> bool {
    let on = |at: Point| {
        theirs.vias.iter().any(|v| v.position == at)
            || (theirs.tracks.iter())
                .flat_map(|t| t.path.windows(2))
                .any(|w| lies_on(w[0], w[1], at))
    };
    (routes.tracks.iter())
        .flat_map(|t| [t.path.first(), t.path.last()])
        .flatten()
        .any(|&at| !pads.contains(&at) && on(at))
        || routes.vias.iter().any(|v| on(v.position))
}

/// The connections that must stay where they are now that `grown`, a new way
/// for `connection`, may start or end on its net's copper beside a pad
/// instead of on the pad: for each pad it leaves, those of the pad's side
/// that still reach the pad, which would otherwise be left alone once they
/// grew elsewhere too. `None` where none does: the new way leaves the pad
/// alone itself.
fn holding_pads_left(
    design: &Design,
    connection: Connection,
    grown: &Routes,
    sides: &[Vec<usize>; 2],
    routed: &[Option<Routes>],
) -> Option<Vec<usize>> {
    let mut holding = Vec::new();
    for (pad, side) in [connection.from, connection.to].into_iter().zip(sides) {
        if reaches_pad(design, grown, pad) {
            continue;
        }
        let reaching: Vec<usize> = (side.iter().copied())
            .filter(|&k| {
                routed[k]
                    .as_ref()
                    .is_some_and(|r| reaches_pad(design, r, pad))
            })
            .collect();
        if reaching.is_empty() {
            return None;
        }
        holding.extend(reaching);
    }
    Some(holding)
}

/// Whether `routes` reach `pad`: at its centre, or at a free end of the
/// lanes of the board's wiring it starts.
fn reaches_pad(design: &Design, routes: &Routes, pad: traxide_contracts::PadId) -> bool {
    let pad = design.pad(pad);
    reaches(routes, pad.position)
        || lanes::free_ends(design, pad)
            .iter()
            .any(|end| reaches(routes, end.at))
}

/// Whether `at` lies on the segment from `a` to `b`.
fn lies_on(a: Point, b: Point, at: Point) -> bool {
    let (ab, aat) = (b - a, at - a);
    let cross = i64::from(ab.x) * i64::from(aat.y) - i64::from(ab.y) * i64::from(aat.x);
    cross == 0
        && (a.x.min(b.x)..=a.x.max(b.x)).contains(&at.x)
        && (a.y.min(b.y)..=a.y.max(b.y)).contains(&at.y)
}

/// Whether a track of `routes` ends at `at`, or a via stands there.
fn reaches(routes: &Routes, at: Point) -> bool {
    routes
        .tracks
        .iter()
        .any(|t| t.path.first() == Some(&at) || t.path.last() == Some(&at))
        || routes.vias.iter().any(|v| v.position == at)
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

    /// Whether it got further than `other`, or as far with fewer vias.
    fn is_better_than(&self, other: &Self, plan: &Plan) -> bool {
        let vias = |attempt: &Self| -> usize {
            (attempt.routed.iter().flatten())
                .map(|routes| routes.vias.len())
                .sum()
        };
        let (mine, theirs) = (self.standing(plan), other.standing(plan));
        mine > theirs || (mine == theirs && vias(self) < vias(other))
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

/// The box around the corners of `routes`' tracks and its vias.
fn copper_bounds(routes: &Routes) -> Option<Rect> {
    Rect::around(
        (routes.tracks.iter())
            .flat_map(|track| track.path.iter().copied())
            .chain(routes.vias.iter().map(|via| via.position)),
    )
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

    fn ways_to_run_layers_of(design: &Design, against_layer_direction: f64) -> Vec<u64> {
        let rules = traxide_rules::compile(
            design,
            Purpose::Routing {
                grid_pitch: 100_000,
            },
        );
        let profile = Profile {
            name: "test".into(),
            grid_pitch: 100_000,
            via_cost: 2_000_000,
            bend_cost: 100_000,
            detour_margin: 5_000_000,
            rip_up_cost: 5_000_000,
            max_rip_ups: 10,
            history_cost: 0,
            heuristic_weight: 1.0,
            global_routing: false,
            batch_size: 1,
            against_layer_direction,
            rounds: 2,
        };
        let ways = Router::new(design, &rules, &profile).ways_to_run_layers();
        assert!(
            ways.iter()
                .all(|way| way.against_per_mille
                    == (against_layer_direction * 1000.0).round() as i64)
        );
        ways.iter().map(|way| way.horizontal_layers).collect()
    }

    #[test]
    fn layers_run_crosswise_through_the_stack_either_way() {
        let design = DesignBuilder::new(30.0, 30.0)
            .layers(4)
            .plane_layer(1)
            .build();
        assert_eq!(ways_to_run_layers_of(&design, 0.5), [0b1001, 0b0100]);
    }

    #[test]
    fn layers_run_no_way_on_a_board_with_one_layer_to_route_on_or_without_a_price() {
        let one = DesignBuilder::new(30.0, 30.0).layers(1).build();
        assert!(ways_to_run_layers_of(&one, 0.5).is_empty());
        let two = DesignBuilder::new(30.0, 30.0).layers(2).build();
        assert!(ways_to_run_layers_of(&two, 0.0).is_empty());
    }

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
            against_layer_direction: 0.0,
            rounds: 2,
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

    fn track_from(path: &[(i32, i32)]) -> Routes {
        Routes {
            tracks: vec![Track {
                net: NetId::from_index(0),
                layer: LayerId(0),
                width: 100_000,
                path: path.iter().map(|&(x, y)| Point::new(x, y)).collect(),
            }],
            vias: Vec::new(),
        }
    }

    #[test]
    fn a_route_that_meets_another_at_its_own_pad_does_not_end_on_it() {
        // Theirs runs from (0, 0) to the pad at (10, 0), where ours starts.
        let pads = [Point::new(10, 0), Point::new(10, 20)];
        let theirs = track_from(&[(0, 0), (10, 0)]);
        assert!(!ends_on(&track_from(&[(10, 0), (10, 20)]), &theirs, pads));
        assert!(ends_on(&track_from(&[(5, 0), (10, 20)]), &theirs, pads));
    }

    #[test]
    fn a_route_of_a_via_alone_ends_on_the_copper_it_stands_on() {
        // Growing trees may join two copper of the net crossing on different
        // layers with a via and no track at all.
        let pads = [Point::new(10, 0), Point::new(10, 20)];
        let theirs = track_from(&[(0, 5), (20, 5)]);
        let via_alone = Routes {
            tracks: Vec::new(),
            vias: vec![Via {
                net: NetId::from_index(0),
                kind: ViaKindId::from_index(0),
                position: Point::new(5, 5),
            }],
        };
        assert!(ends_on(&via_alone, &theirs, pads));
    }

    #[test]
    fn a_route_grown_off_its_pad_holds_the_routes_still_reaching_the_pad() {
        // P1 to P2 grows to start on P3's track to P1 instead of on P1: that
        // track alone reaches P1 then, and must stay.
        let design = DesignBuilder::new(30.0, 30.0)
            .smd("P1", Some("A"), at(10.0, 10.0), 1.0, 0)
            .smd("P2", Some("A"), at(20.0, 10.0), 1.0, 0)
            .smd("P3", Some("A"), at(10.0, 20.0), 1.0, 0)
            .build();
        let pad = |name: &str| design.pad_named(name);
        let mm = traxide_contracts::testing::mm;
        let track = |points: &[(f64, f64)]| Routes {
            tracks: vec![Track {
                net: NetId::from_index(0),
                layer: LayerId(0),
                width: 200_000,
                path: points
                    .iter()
                    .map(|&(x, y)| Point::new(mm(x), mm(y)))
                    .collect(),
            }],
            vias: Vec::new(),
        };
        let connection = Connection {
            net: NetId::from_index(0),
            from: pad("P1"),
            to: pad("P2"),
        };
        let sides = [vec![0], Vec::new()];
        let grown_off_p1 = track(&[(10.0, 15.0), (20.0, 10.0)]);
        let p3_to_p1 = Some(track(&[(10.0, 20.0), (10.0, 10.0)]));
        assert_eq!(
            holding_pads_left(&design, connection, &grown_off_p1, &sides, &[p3_to_p1]),
            Some(vec![0])
        );
        let p3_off_p1 = Some(track(&[(10.0, 20.0), (15.0, 15.0)]));
        assert_eq!(
            holding_pads_left(&design, connection, &grown_off_p1, &sides, &[p3_off_p1]),
            None
        );
        let grown_from_p1 = track(&[(10.0, 10.0), (20.0, 10.0)]);
        assert_eq!(
            holding_pads_left(&design, connection, &grown_from_p1, &sides, &[None]),
            Some(Vec::new())
        );
    }
}
