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
    /// Where to write a DRC report of the stored copper, the board's own
    /// included, in KiCad's JSON format, if anywhere.
    pub drc_report: Option<PathBuf>,
    /// How long negotiation may go on, counted from the start of the job;
    /// what is still queued then routes once more without ripping anything up.
    pub time_limit: Option<Duration>,
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
        Violation::Edge { item, .. } | Violation::Keepout { item, .. } => is_new(item),
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

/// `design` without the planes of nets in `classes`, which get no copper.
fn without_planes_of(design: &Design, classes: &[NetClassId]) -> Design {
    let mut design = design.clone();
    let nets = &design.nets;
    design
        .planes
        .retain(|plane| !classes.contains(&nets[plane.net.index()].class));
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
    let design = traxide_board_access::load_design(&job.input)?;
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
    let joins =
        traxide_planning::join_planes(&without_planes_of(&design, &skipped), &rules, &mut layout);
    let mut layout = Layout::build(&joins.design, &rules);
    let mut plan = traxide_planning::plan(&joins.design);
    plan.connections
        .retain(|c| !skipped.contains(&design.net(c.net).class));
    let escaped = traxide_planning::escape(&joins.design, &rules, &plan, &mut layout);
    // Built anew, the layout holds the escapes as pads of their nets, which
    // rip-up can never take from them.
    let (plan, mut layout) = (&escaped.plan, Layout::build(&escaped.design, &rules));
    let corridors = profile.global_routing.then(|| {
        events.publish(Event::StageStarted(Stage::GlobalRouting));
        traxide_global::route(&escaped.design, plan, &rules, &profile)
    });
    events.publish(Event::StageStarted(Stage::Routing));
    let total = plan.connections.len();
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
    routes.extend(escaped.copper_used(&unrouted));
    routes.extend(routed);
    let verification_rules = traxide_rules::compile(&design, Purpose::Verification);
    let mut total = total;
    if !cancel.is_cancelled() {
        let mut everything = design.existing.clone();
        everything.extend(routes.clone());
        let verdict = traxide_verification::verify(&design, &verification_rules, &everything);
        let to_join = plane_pads_to_join(&design, &verdict);
        if !to_join.is_empty() {
            // Its connections are no part of the plan the layout's routes
            // belong to, so they may rip none of them up.
            let without_rip_up = Profile {
                max_rip_ups: 0,
                ..profile.clone()
            };
            total += to_join.len();
            let joined = Router::new(&escaped.design, &rules, &without_rip_up)
                .with_threads(threads)
                .route_plan(
                    &Plan {
                        connections: to_join,
                    },
                    None,
                    &mut layout,
                    cancel,
                    events,
                );
            routes.extend(joined.routes);
            unrouted.extend(joined.unrouted);
        }
    }
    let routes = routes.without_repeated_vias(&design.existing);
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

#[cfg(test)]
mod tests {
    use super::*;
    use traxide_contracts::testing::{DesignBuilder, at};

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
