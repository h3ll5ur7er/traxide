//! Routing Manager: the workflow of routing a board (volatility V2 of
//! docs/Architecture/Volatilities.md). It decides which stages run in which
//! order; the engines and resource access do the work.

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use traxide_board_access::BoardError;
pub use traxide_board_access::RUNNING_KICAD;
use traxide_contracts::{
    Connection, ConstraintSet, CopperItem, Design, Event, NetClassId, NetId, PadId, Plan, Profile,
    Routes, RoutingResult, Stage, Verdict, Violation,
};
use traxide_detailed::Router;
use traxide_events::{CancelToken, Publisher};
use traxide_layout_access::Layout;
use traxide_profile_access::ProfileError;
use traxide_rules::Purpose;

/// Route the board in `input` and store the routes in `output`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteJob {
    /// A board file, or [`RUNNING_KICAD`] for the board open in KiCad.
    pub input: PathBuf,
    /// Beside the input by default: a session for a Specctra design, a copy of
    /// a KiCad board; back into KiCad for the board open in it.
    pub output: Option<PathBuf>,
    pub profile: String,
    pub profile_file: Option<PathBuf>,
    /// How many threads to route on; as many as the machine has by default.
    pub threads: Option<NonZeroUsize>,
    /// Net classes whose nets get no copper, such as those a plane will join.
    pub unrouted_net_classes: Vec<String>,
    pub nets: Nets,
    /// Where to write a DRC report of the stored copper, the board's own
    /// included, in KiCad's JSON format, if anywhere.
    pub drc_report: Option<PathBuf>,
    /// How long negotiation may go on, counted from the start of the job;
    /// what is still queued then routes once more without ripping anything up.
    pub time_limit: Option<Duration>,
    /// Route the board's own wiring again, but for what the designer locked
    /// or protected: a designer's tracks are moved only when asked.
    pub reroute_existing: bool,
}

/// Which nets a job routes; the rest of the board is kept as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nets {
    All,
    /// These, by name.
    Named(Vec<String>),
    /// Those of what is selected in KiCad's PCB editor, for the board open
    /// there: the copper selected and the selected footprints' pads.
    SelectedInKicad,
}

/// What a finished route job achieved.
#[derive(Debug, Clone)]
pub struct RouteReport {
    pub connections: usize,
    /// Each as the net's name and the names of its two pads.
    pub unrouted: Vec<Unrouted>,
    pub tracks: usize,
    pub vias: usize,
    /// The independent check of the stored routes. Unconnected pads follow from
    /// unrouted connections; any other violation is a bug of the router.
    pub verdict: Verdict,
    /// Rules the board broke before routing, between its pads and the copper it
    /// came with, which routing leaves as they are.
    pub violations_before_routing: usize,
    pub cancelled: bool,
    pub elapsed: Duration,
    /// Where the routes were stored.
    pub output: PathBuf,
    /// The running KiCad refilled the board's copper zones around the routes,
    /// an undo step of its own: taking the routes back takes two undos.
    pub zones_refilled: bool,
    /// The board file keeps the fills its copper zones came with: the
    /// designer refills them in KiCad.
    pub zones_to_refill: bool,
    /// The names of the layers kept free of tracks, as planes.
    pub planes: Vec<String>,
    /// What the reader found wrong with the board and read anyway.
    pub warnings: Vec<String>,
}

/// A connection routing could not make, by the names of its net and pads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unrouted {
    pub net: String,
    pub from: String,
    pub to: String,
    /// What blocks each of its pads that no track can leave, or else that no
    /// way was found between them.
    pub why: Vec<Unroutable>,
}

/// Why a connection stayed unrouted, so that the designer can act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unroutable {
    /// This pad lies outside the board's outline.
    OffTheBoard(String),
    /// This pad has copper only on layers routing may not use, such as planes.
    OnlyOnLayersNotRouted(String),
    /// Copper of another net, a keepout or the board's edge is too close to
    /// this pad for any track to leave it.
    HemmedIn(String),
    /// The tracks of other connections took every way out of this pad; routed
    /// in another order, it may have been left.
    ExitsTaken(String),
    /// The board's own copper, keepouts and edge leave no way between the
    /// pads, whatever routing does.
    CutOff,
    /// Both pads can be left, but no way between them was found.
    NoWayFound,
}

#[derive(Debug, thiserror::Error)]
pub enum RouteError {
    #[error(transparent)]
    Board(#[from] BoardError),
    #[error(transparent)]
    Profile(#[from] ProfileError),
    #[error("no net class named {name}; there are {}", known.join(", "))]
    UnknownNetClass { name: String, known: Vec<String> },
    #[error("the board has no net named {name}")]
    UnknownNet { name: String },
    #[error(
        "routing the board's own wiring again works on a board file or a Specctra design; \
         in a running KiCad, delete the tracks to route again instead"
    )]
    RerouteInRunningKicad,
    #[error("only the board open in KiCad has a selection to route the nets of")]
    SelectionOutsideKicad,
}

fn why_unrouted(
    design: &Design,
    connection: &Connection,
    hemmed_in: &[PadId],
    exits_taken: &[PadId],
    cut_off: &[Connection],
) -> Vec<Unroutable> {
    let blocked: Vec<Unroutable> = [connection.from, connection.to]
        .into_iter()
        .filter_map(|pad| {
            if hemmed_in.contains(&pad) {
                Some(why_hemmed_in(design, pad))
            } else {
                exits_taken
                    .contains(&pad)
                    .then(|| Unroutable::ExitsTaken(design.pad(pad).name.clone()))
            }
        })
        .collect();
    if !blocked.is_empty() {
        blocked
    } else if cut_off.contains(connection) {
        vec![Unroutable::CutOff]
    } else {
        vec![Unroutable::NoWayFound]
    }
}

/// What keeps every track from leaving `pad`.
fn why_hemmed_in(design: &Design, pad: PadId) -> Unroutable {
    let pad = design.pad(pad);
    if !design.outline.contains(pad.position) {
        Unroutable::OffTheBoard(pad.name.clone())
    } else if pad
        .copper
        .iter()
        .all(|c| !design.layers[c.layer.index()].routable)
    {
        Unroutable::OnlyOnLayersNotRouted(pad.name.clone())
    } else {
        Unroutable::HemmedIn(pad.name.clone())
    }
}

/// Whether `violation` involves copper routing added, which comes after the
/// design's existing copper in the verified routes. Unconnected pads always do:
/// connecting them is routing's job.
fn involves_new_copper(violation: &Violation, existing: &Routes) -> bool {
    let is_new = |item: &CopperItem| match *item {
        CopperItem::Pad(_) => false,
        CopperItem::Track(index) => index >= existing.tracks.len(),
        CopperItem::Via(index) => index >= existing.vias.len(),
    };
    match violation {
        Violation::Clearance { a, b, .. } | Violation::HoleToHole { a, b, .. } => {
            is_new(a) || is_new(b)
        }
        Violation::TrackWidth { item, .. }
        | Violation::ViaDiameter { item, .. }
        | Violation::HoleSize { item, .. }
        | Violation::Edge { item, .. }
        | Violation::Keepout { item, .. }
        | Violation::Dangling { item, .. } => is_new(item),
        Violation::Unconnected { .. } => true,
    }
}

fn net_classes_named(design: &Design, names: &[String]) -> Result<Vec<NetClassId>, RouteError> {
    let classes = &design.rules.classes;
    names
        .iter()
        .map(|name| {
            classes
                .iter()
                .position(|class| &class.name == name)
                .map(NetClassId::from_index)
                .ok_or_else(|| RouteError::UnknownNetClass {
                    name: name.clone(),
                    known: classes.iter().map(|c| c.name.clone()).collect(),
                })
        })
        .collect()
}

/// Which of the design's nets to route, by index.
fn nets_to_route(design: &Design, nets: &Nets) -> Result<Vec<bool>, RouteError> {
    let names = match nets {
        Nets::All => return Ok(vec![true; design.nets.len()]),
        Nets::Named(names) => names,
        Nets::SelectedInKicad => &traxide_board_access::nets_selected_in_kicad()?,
    };
    let mut chosen = vec![false; design.nets.len()];
    for name in names {
        let index = (design.nets.iter())
            .position(|net| &net.name == name)
            .ok_or_else(|| RouteError::UnknownNet { name: name.clone() })?;
        chosen[index] = true;
    }
    Ok(chosen)
}

/// Optimise the board's own wiring in `input` and store it in `output`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimiseJob {
    /// A KiCad board file, or [`RUNNING_KICAD`] for the board open in KiCad.
    pub input: PathBuf,
    /// Beside the input by default, `BOARD.optimised.kicad_pcb`; back into
    /// KiCad for the board open in it.
    pub output: Option<PathBuf>,
}

/// What optimising did to the board's unlocked segments and vias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimiseReport {
    pub segments_before: usize,
    pub segments_after: usize,
    /// The length of their track, in nanometres.
    pub length_before: i64,
    pub length_after: i64,
    /// Whether the optimised wiring was stored: not where it would break a
    /// rule the board did not break, nor where it changed nothing.
    pub stored: bool,
    /// The independent check of the board as stored, or as it was where
    /// nothing was stored.
    pub verdict: Verdict,
    pub output: PathBuf,
    /// The running KiCad refilled the board's copper zones: taking the
    /// optimisation back takes two undos.
    pub zones_refilled: bool,
    /// The board file keeps the fills its zones came with.
    pub zones_to_refill: bool,
    pub elapsed: Duration,
}

/// Makes the board's own unlocked wiring shorter and simpler (UC4): its
/// segments, chained where nothing else joins their net, pulled tight among
/// the rest of the board; arcs, locked copper and the tracks of differential
/// pairs stay as they are. Stored
/// only where it changes something and breaks no rule the board did not
/// break before, judged by the Verification Engine.
pub fn optimise(
    job: &OptimiseJob,
    events: &impl Publisher<Event>,
    cancel: &CancelToken,
) -> Result<OptimiseReport, RouteError> {
    let started = Instant::now();
    events.publish(Event::StageStarted(Stage::Loading));
    let design = traxide_board_access::load_design(&job.input)?;
    let profile = traxide_profile_access::load_profile("default", None)?;
    events.publish(Event::StageStarted(Stage::CompilingRules));
    let rules = traxide_rules::compile(
        &design,
        Purpose::Routing {
            grid_pitch: profile.grid_pitch,
        },
    );
    let verification_rules = traxide_rules::compile(&design, Purpose::Verification);
    // A differential pair's tracks run side by side, which pulling each tight
    // on its own would undo: they stay as they are, as arcs and locked copper.
    let paired = |net: NetId| rules.pair_of(net).is_some();
    let unlocked = Routes {
        tracks: (design.existing.tracks.iter())
            .filter(|t| !design.locked.tracks.contains(t))
            .cloned()
            .collect(),
        vias: (design.existing.vias.iter())
            .filter(|v| !design.locked.vias.contains(v))
            .cloned()
            .collect(),
    };
    let split = |of_a_pair: bool| Routes {
        tracks: (unlocked.tracks.iter())
            .filter(|t| paired(t.net) == of_a_pair)
            .cloned()
            .collect(),
        vias: (unlocked.vias.iter())
            .filter(|v| paired(v.net) == of_a_pair)
            .cloned()
            .collect(),
    };
    let (of_pairs, rest) = (split(true), split(false));
    let arcs: Vec<_> = (rest.tracks.iter())
        .filter(|t| t.path.len() > 2)
        .cloned()
        .collect();
    let segments = |routes: &Routes| -> usize {
        (routes.tracks.iter())
            .filter(|t| !arcs.contains(t))
            .map(|t| t.path.len().saturating_sub(1))
            .sum()
    };
    let length = |routes: &Routes| -> i64 {
        (routes.tracks.iter())
            .filter(|t| !arcs.contains(t))
            .flat_map(|t| t.path.windows(2))
            .map(|w| w[0].distance(w[1]))
            .sum()
    };
    let mut as_they_are = design.locked.clone();
    as_they_are.extend(of_pairs.clone());
    let mut kept = as_they_are.clone();
    kept.tracks.extend(arcs.iter().cloned());
    events.publish(Event::StageStarted(Stage::Optimising));
    let mut fixed = design.clone();
    fixed.existing = as_they_are;
    let mut layout = Layout::build(&fixed, &rules);
    let optimised = traxide_optimization::optimise(&design, &rest, &mut layout);
    let arc_pieces: Vec<_> = (arcs.iter())
        .flat_map(|arc| {
            arc.path.windows(2).map(|w| traxide_contracts::Track {
                path: vec![w[0], w[1]],
                ..arc.clone()
            })
        })
        .collect();
    let optimised = Routes {
        tracks: (optimised.tracks.into_iter())
            .filter(|t| !arc_pieces.contains(t))
            .collect(),
        vias: optimised.vias,
    };
    events.publish(Event::StageStarted(Stage::Verifying));
    let before = traxide_verification::verify(&design, &verification_rules, &design.existing);
    let mut everything = kept.clone();
    everything.extend(optimised.clone());
    let after = traxide_verification::verify(&design, &verification_rules, &everything);
    let output = job.output.clone().unwrap_or_else(|| {
        if job.input.as_os_str() == RUNNING_KICAD {
            job.input.clone()
        } else {
            job.input.with_extension("optimised.kicad_pcb")
        }
    });
    let (segments_before, length_before) = (segments(&rest), length(&rest));
    let (segments_after, length_after) = (segments(&optimised), length(&optimised));
    let improves = (segments_after, length_after) != (segments_before, length_before);
    let breaks_nothing_new = after.violations.len() <= before.violations.len();
    let store = improves && breaks_nothing_new && !cancel.is_cancelled();
    let mut report = OptimiseReport {
        segments_before,
        segments_after,
        length_before,
        length_after,
        stored: store,
        verdict: if store { after } else { before },
        output: output.clone(),
        zones_refilled: false,
        zones_to_refill: false,
        elapsed: Duration::ZERO,
    };
    if store {
        events.publish(Event::StageStarted(Stage::Storing));
        // In place of all the board's unlocked segments and vias: the pairs'
        // among them go back as they were.
        let mut wiring = optimised.clone();
        wiring.extend(Routes {
            tracks: (of_pairs.tracks.iter())
                .filter(|t| t.path.len() == 2)
                .cloned()
                .collect(),
            vias: of_pairs.vias.clone(),
        });
        let stored =
            traxide_board_access::store_in_place_of_wiring(&output, &job.input, &design, &wiring)?;
        report.zones_refilled = stored.zones_refilled;
        report.zones_to_refill = stored.zones_to_refill;
    }
    report.elapsed = started.elapsed();
    Ok(report)
}

/// `design` with `routes` as its own wiring.
fn with_wiring(design: &Design, routes: &Routes) -> Design {
    let mut design = design.clone();
    design.existing.extend(routes.clone());
    design
}

/// `design` with the planes only of the nets routed, as the others get no
/// copper.
fn with_planes_of(design: &Design, is_routed: impl Fn(NetId) -> bool) -> Design {
    let mut design = design.clone();
    design.planes.retain(|plane| is_routed(plane.net));
    design
}

/// Routes every connection of the board in `job`, storing what fits.
pub fn route(
    job: &RouteJob,
    events: &impl Publisher<Event>,
    cancel: &CancelToken,
) -> Result<RouteReport, RouteError> {
    let started = Instant::now();
    let profile = traxide_profile_access::load_profile(&job.profile, job.profile_file.as_deref())?;
    events.publish(Event::StageStarted(Stage::Loading));
    if job.nets == Nets::SelectedInKicad && job.input.as_os_str() != RUNNING_KICAD {
        return Err(RouteError::SelectionOutsideKicad);
    }
    let mut design = traxide_board_access::load_design(&job.input)?;
    if job.reroute_existing {
        if job.input.as_os_str() == RUNNING_KICAD {
            return Err(RouteError::RerouteInRunningKicad);
        }
        design.existing = design.locked.clone();
    }
    events.publish(Event::StageStarted(Stage::CompilingRules));
    let rules = traxide_rules::compile(
        &design,
        Purpose::Routing {
            grid_pitch: profile.grid_pitch,
        },
    );
    events.publish(Event::StageStarted(Stage::BuildingLayout));
    let mut layout = Layout::build(&design, &rules);
    events.publish(Event::StageStarted(Stage::Planning));
    let skipped = net_classes_named(&design, &job.unrouted_net_classes)?;
    let chosen = nets_to_route(&design, &job.nets)?;
    let is_routed = |net: NetId| chosen[net.index()] && !skipped.contains(&design.net(net).class);
    let joins =
        traxide_planning::join_planes(&with_planes_of(&design, is_routed), &rules, &mut layout);
    let mut layout = Layout::build(&joins.design, &rules);
    let mut plan = traxide_planning::plan(&joins.design);
    plan.connections.retain(|c| is_routed(c.net));
    // Differential pairs route first, side by side, and stay as the board's
    // own wiring would: no connection is planned where they join the pads,
    // and nothing rips them up.
    let coupled = traxide_planning::coupled(&joins.design, &rules, &plan);
    let mut paired = Routes::default();
    let mut connections_paired = 0;
    for routes in
        traxide_detailed::route_pairs(&joins.design, &rules, &profile, &coupled, &mut layout)
            .into_iter()
            .flatten()
    {
        connections_paired += 2;
        paired.extend(routes);
    }
    let with_pairs = with_wiring(&joins.design, &paired);
    if connections_paired > 0 {
        layout = Layout::build(&with_pairs, &rules);
        plan = traxide_planning::plan(&with_pairs);
        plan.connections.retain(|c| is_routed(c.net));
    }
    let escaped = traxide_planning::escape(&with_pairs, &rules, &plan, &mut layout);
    // Built anew, the layout holds the escapes as pads of their nets, which
    // rip-up can never take from them.
    let (plan, mut layout) = (&escaped.plan, Layout::build(&escaped.design, &rules));
    let corridors = profile.global_routing.then(|| {
        events.publish(Event::StageStarted(Stage::GlobalRouting));
        traxide_global::route(&escaped.design, plan, &rules, &profile)
    });
    events.publish(Event::StageStarted(Stage::Routing));
    let total = plan.connections.len() + connections_paired;
    let threads = job
        .threads
        .or_else(|| std::thread::available_parallelism().ok())
        .map_or(1, NonZeroUsize::get);
    let mut router = Router::new(&escaped.design, &rules, &profile).with_threads(threads);
    if let Some(limit) = job.time_limit {
        router = router.with_deadline(started + limit);
    }
    let RoutingResult {
        routes: routed,
        mut unrouted,
        hemmed_in,
        exits_taken,
        cut_off,
    } = router.route_plan(plan, corridors.as_ref(), &mut layout, cancel, events);
    let mut routes = joins.copper.clone();
    routes.extend(escaped.copper_used(&unrouted, &routed));
    routes.extend(routed);
    let verification_rules = traxide_rules::compile(&design, Purpose::Verification);
    let mut total = total;
    if !cancel.is_cancelled() {
        let mut everything = design.existing.clone();
        everything.extend(paired.clone());
        everything.extend(routes.clone());
        let verdict = traxide_verification::verify(&design, &verification_rules, &everything);
        let mut to_join = plane_pads_to_join(&design, &verdict);
        to_join.retain(|c| is_routed(c.net));
        if !to_join.is_empty() {
            total += to_join.len();
            let joined = join_among(
                &escaped.design,
                &routes,
                to_join,
                (&rules, &profile, threads),
                cancel,
                events,
            );
            routes.extend(joined.routes);
            unrouted.extend(joined.unrouted);
        }
    }
    let mut routes = routes.without_repeated_vias(&design.existing);
    if !cancel.is_cancelled() {
        events.publish(Event::StageStarted(Stage::Optimising));
        // The pairs stay as routed: pulled tight one at a time, their
        // tracks would no longer run side by side.
        let with_paired = with_wiring(&design, &paired);
        routes =
            traxide_optimization::pull_tight(&routes, &mut Layout::build(&with_paired, &rules));
    }
    routes.extend(paired);
    let at_pins = |connections: Vec<Connection>| -> Vec<Connection> {
        connections
            .into_iter()
            .map(|c| escaped.at_pins(c))
            .collect()
    };
    let at_pin = |pads: Vec<PadId>| -> Vec<PadId> {
        pads.into_iter().map(|p| escaped.original(p)).collect()
    };
    let (unrouted, cut_off) = (at_pins(unrouted), at_pins(cut_off));
    let (hemmed_in, exits_taken) = (at_pin(hemmed_in), at_pin(exits_taken));
    events.publish(Event::StageStarted(Stage::Verifying));
    let mut everything = design.existing.clone();
    everything.extend(routes.clone());
    let full_verdict = traxide_verification::verify(&design, &verification_rules, &everything);
    events.publish(Event::StageStarted(Stage::Storing));
    let output = job
        .output
        .clone()
        .unwrap_or_else(|| traxide_board_access::default_routes_path(&job.input));
    let stored = traxide_board_access::store_routes(&output, &job.input, &design, &routes)?;
    if let Some(drc_report) = &job.drc_report {
        traxide_board_access::store_drc_report(
            drc_report,
            &output,
            &design,
            &everything,
            &full_verdict,
        )?;
    }
    let (violations, before_routing): (Vec<Violation>, Vec<Violation>) = full_verdict
        .violations
        .into_iter()
        .partition(|v| involves_new_copper(v, &design.existing));
    let verdict = Verdict { violations };
    let named = |c: &Connection| Unrouted {
        net: design.net(c.net).name.clone(),
        from: design.pad(c.from).name.clone(),
        to: design.pad(c.to).name.clone(),
        why: why_unrouted(&design, c, &hemmed_in, &exits_taken, &cut_off),
    };
    Ok(RouteReport {
        connections: total,
        unrouted: unrouted.iter().map(named).collect(),
        tracks: routes.tracks.len(),
        vias: routes.vias.len(),
        verdict,
        violations_before_routing: before_routing.len(),
        cancelled: cancel.is_cancelled(),
        elapsed: started.elapsed(),
        output,
        zones_refilled: stored.zones_refilled,
        zones_to_refill: stored.zones_to_refill,
        planes: design
            .layers
            .iter()
            .filter(|layer| !layer.routable)
            .map(|layer| layer.name.clone())
            .collect(),
        warnings: [
            design.warnings.clone(),
            pads_too_close(&design, &verification_rules),
        ]
        .concat(),
    })
}

/// Warns of each pair of pads of different nets closer than their clearance,
/// which verification leaves to the footprints and no routing can fix.
fn pads_too_close(design: &Design, rules: &ConstraintSet) -> Vec<String> {
    let named = |pad| {
        let pad = design.pad(pad);
        let net = pad.net.map_or("", |net| design.net(net).name.as_str());
        format!("{} (net {net})", pad.name)
    };
    traxide_verification::pads_too_close(design, rules)
        .into_iter()
        .map(|[a, b]| {
            format!(
                "the pads {} and {} are closer than their clearance",
                named(a),
                named(b)
            )
        })
        .collect()
}

/// The connections that join, by track, the pads of a net with a plane that
/// the plane's fill does not reach, as other nets' copper walls their join
/// off: each to the nearest pad of its net that is joined (TRX-67).
fn plane_pads_to_join(design: &Design, verdict: &Verdict) -> Vec<Connection> {
    let has_a_plane = |net: NetId| {
        design
            .planes
            .iter()
            .any(|p| p.net == net && !design.layers[p.layer.index()].routable)
    };
    let mut connections = Vec::new();
    for violation in &verdict.violations {
        let Violation::Unconnected { net, pads: apart } = violation else {
            continue;
        };
        if !has_a_plane(*net) {
            continue;
        }
        let joined: Vec<PadId> = (design.net(*net).pads.iter())
            .copied()
            .filter(|pad| !apart.contains(pad))
            .collect();
        for &pad in apart {
            let at = design.pad(pad).position;
            let nearest =
                (joined.iter().copied()).min_by_key(|&j| design.pad(j).position.distance(at));
            if let Some(to) = nearest {
                connections.push(Connection {
                    net: *net,
                    from: pad,
                    to,
                });
            }
        }
    }
    connections
}

/// Routes `connections` among `routes`, which stay as they are: a layout of
/// its own holds them as the board's copper. Sharing the routing's layout, a
/// plan of other connections would take its routes for theirs by their ids.
fn join_among(
    design: &Design,
    routes: &Routes,
    connections: Vec<Connection>,
    (rules, profile, threads): (&ConstraintSet, &Profile, usize),
    cancel: &CancelToken,
    events: &impl Publisher<Event>,
) -> RoutingResult {
    let mut settled = design.clone();
    settled.existing.extend(routes.clone());
    let mut layout = Layout::build(&settled, rules);
    Router::new(&settled, rules, profile)
        .with_threads(threads)
        .route_plan(&Plan { connections }, None, &mut layout, cancel, events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use traxide_contracts::testing::{DesignBuilder, at};

    #[test]
    fn pads_joined_after_routing_keep_clear_of_the_routes_before_them() {
        let design = DesignBuilder::new(20.0, 10.0)
            .layers(3)
            .plane_layer(1)
            .smd("G2", Some("GND"), at(10.0, 5.0), 1.0, 0)
            .smd("G3", Some("GND"), at(16.0, 5.0), 1.0, 0)
            .smd("A1", Some("A"), at(13.0, 9.0), 1.0, 0)
            .smd("A2", Some("A"), at(13.0, 1.0), 1.0, 0)
            .build();
        let across = Routes {
            tracks: vec![traxide_contracts::Track {
                net: design.net_named("A"),
                layer: traxide_contracts::LayerId::from_index(0),
                width: 200_000,
                path: vec![at(13.0, 9.0), at(13.0, 1.0)],
            }],
            vias: vec![],
        };
        let to_join = vec![Connection {
            net: design.net_named("GND"),
            from: design.pad_named("G3"),
            to: design.pad_named("G2"),
        }];
        let rules = traxide_rules::compile(
            &design,
            Purpose::Routing {
                grid_pitch: 100_000,
            },
        );
        let profile = traxide_profile_access::load_profile("default", None).expect("profile");
        let joined = join_among(
            &design,
            &across,
            to_join,
            (&rules, &profile, 1),
            &CancelToken::new(),
            &traxide_events::Silent,
        );
        assert!(joined.unrouted.is_empty());
        let mut everything = across.clone();
        everything.extend(joined.routes);
        let rules = traxide_rules::compile(&design, Purpose::Verification);
        let verdict = traxide_verification::verify(&design, &rules, &everything);
        let broken: Vec<_> = (verdict.violations.iter())
            .filter(|v| !matches!(v, Violation::Unconnected { .. }))
            .collect();
        assert!(broken.is_empty(), "{broken:?}");
    }

    #[test]
    fn a_pad_walled_off_from_its_plane_is_joined_by_track_to_the_nearest_joined_pad() {
        let design = DesignBuilder::new(20.0, 10.0)
            .layers(3)
            .plane_layer(1)
            .plane("GND", 1, at(0.0, 0.0), at(20.0, 10.0))
            .smd("G1", Some("GND"), at(2.0, 5.0), 1.0, 0)
            .smd("G2", Some("GND"), at(12.0, 5.0), 1.0, 0)
            .smd("G3", Some("GND"), at(15.0, 5.0), 1.0, 0)
            .smd("A1", Some("A"), at(2.0, 8.0), 1.0, 0)
            .smd("A2", Some("A"), at(18.0, 8.0), 1.0, 0)
            .build();
        let gnd = design.net_named("GND");
        let verdict = Verdict {
            violations: vec![
                Violation::Unconnected {
                    net: gnd,
                    pads: vec![design.pad_named("G3")],
                },
                // A net without a plane is left to routing.
                Violation::Unconnected {
                    net: design.net_named("A"),
                    pads: vec![design.pad_named("A2")],
                },
            ],
        };
        assert_eq!(
            plane_pads_to_join(&design, &verdict),
            [Connection {
                net: gnd,
                from: design.pad_named("G3"),
                to: design.pad_named("G2"),
            }]
        );
    }
}
