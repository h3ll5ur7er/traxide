use traxide_contracts::geometry::{Point, Polygon, Rect, Segment, Shape};
use traxide_contracts::testing::{DesignBuilder, at, mm};
use traxide_contracts::{
    Connection, ConnectionId, Corridors, Design, Event, KeepoutScope, LayerId, LayoutAccess, NetId,
    Obstruction, Plan, Profile, Routes, RoutingResult, Violation,
};
use traxide_detailed::Router;
use traxide_events::{CancelToken, Silent};
use traxide_layout_access::Layout;
use traxide_rules::Purpose;

fn profile() -> Profile {
    Profile {
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
        against_layer_direction: 0.0,
    }
}

fn connection(design: &Design, from: &str, to: &str) -> Connection {
    let from = design.pad_named(from);
    Connection {
        net: design.pad(from).net.expect("pads of a net"),
        from,
        to: design.pad_named(to),
    }
}

/// Routes `pairs` as a plan, in that order.
fn route_with(design: &Design, profile: &Profile, pairs: &[(&str, &str)]) -> RoutingResult {
    route_in(design, profile, pairs, None)
}

fn route_in(
    design: &Design,
    profile: &Profile,
    pairs: &[(&str, &str)],
    corridors: Option<&Corridors>,
) -> RoutingResult {
    route_on_threads(design, profile, pairs, corridors, 1)
}

fn route_on_threads(
    design: &Design,
    profile: &Profile,
    pairs: &[(&str, &str)],
    corridors: Option<&Corridors>,
    threads: usize,
) -> RoutingResult {
    let rules = traxide_rules::compile(
        design,
        Purpose::Routing {
            grid_pitch: 100_000,
        },
    );
    let mut layout = Layout::build(design, &rules);
    let connections = pairs
        .iter()
        .map(|&(from, to)| connection(design, from, to))
        .collect();
    let plan = Plan { connections };
    let cancel = CancelToken::new();
    Router::new(design, &rules, profile)
        .with_threads(threads)
        .route_plan(&plan, corridors, &mut layout, &cancel, &Silent)
}

/// A corridor of 1 mm cells on a board 20 cells wide, made of the given cells.
fn corridor(cells: impl IntoIterator<Item = (i32, i32)>) -> Corridors {
    let mut cells: Vec<u32> = cells
        .into_iter()
        .map(|(column, row)| (row * 20 + column) as u32)
        .collect();
    cells.sort_unstable();
    cells.dedup();
    Corridors::new(Point::new(0, 0), 1_000_000, 20, vec![cells])
}

fn route(design: &Design, pairs: &[(&str, &str)]) -> RoutingResult {
    route_with(design, &profile(), pairs)
}

/// The routes break no rule, with the board's own wiring, judged by the
/// independent Verification Engine.
fn assert_legal(design: &Design, routes: &Routes) {
    let rules = traxide_rules::compile(design, Purpose::Verification);
    let mut everything = design.existing.clone();
    everything.extend(routes.clone());
    let verdict = traxide_verification::verify(design, &rules, &everything);
    let broken: Vec<_> = verdict
        .violations
        .iter()
        .filter(|v| !matches!(v, Violation::Unconnected { .. }))
        .collect();
    assert!(broken.is_empty(), "{broken:?}");
}

fn wall(min: Point, max: Point) -> Shape {
    Shape::Polygon(Polygon::rectangle(Rect { min, max }))
}

fn is_octilinear(a: Point, b: Point) -> bool {
    let (dx, dy) = ((b.x - a.x).abs(), (b.y - a.y).abs());
    dx == 0 || dy == 0 || dx == dy
}

#[test]
fn a_clear_line_is_one_straight_track() {
    let design = DesignBuilder::new(20.0, 10.0)
        .smd("A1", Some("A"), at(5.0, 5.0), 1.0, 0)
        .smd("A2", Some("A"), at(15.0, 5.0), 1.0, 0)
        .build();
    let routes = route(&design, &[("A1", "A2")]).routes;
    assert_eq!(routes.vias, []);
    assert_eq!(routes.tracks.len(), 1);
    assert_eq!(routes.tracks[0].layer, LayerId(0));
    assert_eq!(routes.tracks[0].path, [at(5.0, 5.0), at(15.0, 5.0)]);
}

#[test]
fn routes_around_a_pad_of_another_net() {
    let design = DesignBuilder::new(20.0, 10.0)
        .smd("A1", Some("A"), at(5.0, 5.0), 1.0, 0)
        .smd("A2", Some("A"), at(15.0, 5.0), 1.0, 0)
        .smd("B1", Some("B"), at(10.0, 5.0), 2.0, 0)
        .without_vias()
        .build();
    let result = route(&design, &[("A1", "A2")]);
    assert!(result.unrouted.is_empty());
    assert!(result.routes.tracks[0].path.len() > 2, "goes around");
    assert_legal(&design, &result.routes);
}

#[test]
fn changes_layers_to_pass_a_wall_on_the_top() {
    let design = DesignBuilder::new(20.0, 10.0)
        .smd("A1", Some("A"), at(5.0, 5.0), 1.0, 0)
        .smd("A2", Some("A"), at(15.0, 5.0), 1.0, 0)
        .keepout(
            wall(at(9.5, 0.0), at(10.5, 10.0)),
            Some(0),
            KeepoutScope::Everything,
        )
        .build();
    let routes = route(&design, &[("A1", "A2")]).routes;
    assert_eq!(routes.vias.len(), 2);
    assert!(routes.tracks.iter().any(|t| t.layer == LayerId(1)));
    assert_legal(&design, &routes);
}

#[test]
fn changes_layers_through_vias_of_its_net_the_board_came_with() {
    // A wall on the top, and vias kept out everywhere but around two vias of
    // the net off the grid, too close to them for a via of its own.
    let mut builder = DesignBuilder::new(20.0, 10.0)
        .smd("A1", Some("A"), at(5.0, 5.0), 1.0, 0)
        .smd("A2", Some("A"), at(15.0, 5.0), 1.0, 0)
        .keepout(
            wall(at(9.5, 0.0), at(10.5, 10.0)),
            Some(0),
            KeepoutScope::Everything,
        );
    let no_vias = [
        ((0.0, 0.0), (7.5, 10.0)),
        ((8.6, 0.0), (11.5, 10.0)),
        ((12.6, 0.0), (20.0, 10.0)),
        ((7.5, 0.0), (8.6, 4.5)),
        ((7.5, 5.6), (8.6, 10.0)),
        ((11.5, 0.0), (12.6, 4.5)),
        ((11.5, 5.6), (12.6, 10.0)),
    ];
    for ((x0, y0), (x1, y1)) in no_vias {
        builder = builder.keepout(wall(at(x0, y0), at(x1, y1)), None, KeepoutScope::ViasOnly);
    }
    let mut design = builder.build();
    design.rules.hole_to_hole = mm(0.25);
    for x in [8.03, 12.03] {
        design.existing.vias.push(traxide_contracts::Via {
            net: design.net_named("A"),
            kind: traxide_contracts::ViaKindId(0),
            position: at(x, 5.04),
        });
    }
    let result = route(&design, &[("A1", "A2")]);
    assert!(result.unrouted.is_empty());
    assert_eq!(result.routes.vias, []);
    assert!(result.routes.tracks.iter().any(|t| t.layer == LayerId(1)));
    let mut everything = design.existing.clone();
    everything.extend(result.routes);
    let rules = traxide_rules::compile(&design, Purpose::Verification);
    let verdict = traxide_verification::verify(&design, &rules, &everything);
    assert_eq!(verdict.violations, []);
}

#[test]
fn a_path_keeps_its_own_vias_the_hole_to_hole_distance_apart() {
    // A thin wall keeps tracks off the bottom at x = 10 mm, and the top takes
    // tracks only from 9.6 to 10.4 mm: the hop over the wall is short, and its
    // vias may come as close as 0.4 mm.
    let mut design = DesignBuilder::new(20.0, 10.0)
        .smd("A1", Some("A"), at(5.0, 5.0), 1.0, 1)
        .smd("A2", Some("A"), at(15.0, 5.0), 1.0, 1)
        .keepout(
            wall(at(9.95, 0.0), at(10.05, 10.0)),
            Some(1),
            KeepoutScope::TracksOnly,
        )
        .keepout(
            wall(at(0.0, 0.0), at(9.6, 10.0)),
            Some(0),
            KeepoutScope::TracksOnly,
        )
        .keepout(
            wall(at(10.4, 0.0), at(20.0, 10.0)),
            Some(0),
            KeepoutScope::TracksOnly,
        )
        .build();
    design.rules.hole_to_hole = mm(0.25);
    let result = route(&design, &[("A1", "A2")]);
    assert!(result.unrouted.is_empty());
    assert_eq!(result.routes.vias.len(), 2, "{:?}", result.routes.vias);
    assert_legal(&design, &result.routes);
}

#[test]
fn a_route_continues_a_fixed_lane_from_its_free_end_instead_of_leaving_it_dangling() {
    // The lane leads from A1 towards A2, but off the line between them.
    let mut design = DesignBuilder::new(20.0, 10.0)
        .smd("A1", Some("A"), at(5.0, 5.0), 1.0, 0)
        .smd("A2", Some("A"), at(9.0, 9.5), 1.0, 0)
        .build();
    design.existing.tracks.push(traxide_contracts::Track {
        net: design.net_named("A"),
        layer: LayerId(0),
        width: mm(0.2),
        path: vec![at(5.0, 5.0), at(5.0, 8.0)],
    });
    let result = route(&design, &[("A1", "A2")]);
    assert!(result.unrouted.is_empty());
    let ends: Vec<Point> = result
        .routes
        .tracks
        .iter()
        .flat_map(|t| [t.path[0], t.path[t.path.len() - 1]])
        .collect();
    assert!(ends.contains(&at(5.0, 8.0)), "{:?}", result.routes.tracks);
    assert_legal(&design, &result.routes);
}

#[test]
fn a_route_continues_a_fixed_lane_that_leads_away_from_its_target() {
    // The lane leads from A1 away from A2: joining A1 straight is shorter.
    let mut design = DesignBuilder::new(20.0, 10.0)
        .smd("A1", Some("A"), at(8.0, 5.0), 1.0, 0)
        .smd("A2", Some("A"), at(14.0, 5.0), 1.0, 0)
        .build();
    design.existing.tracks.push(traxide_contracts::Track {
        net: design.net_named("A"),
        layer: LayerId(0),
        width: mm(0.2),
        path: vec![at(8.0, 5.0), at(5.0, 5.0)],
    });
    let result = route(&design, &[("A1", "A2")]);
    assert!(result.unrouted.is_empty());
    let ends: Vec<Point> = result
        .routes
        .tracks
        .iter()
        .flat_map(|t| [t.path[0], t.path[t.path.len() - 1]])
        .collect();
    assert!(ends.contains(&at(5.0, 5.0)), "{:?}", result.routes.tracks);
    assert_legal(&design, &result.routes);
}

#[test]
fn a_route_goes_round_a_wall_rather_than_under_it_when_the_way_round_is_short() {
    // Round the wall's end is some 5.4 mm further: less than two vias at the
    // dearer price routing is tidied at, more than two at the price it
    // negotiates at.
    let design = DesignBuilder::new(20.0, 10.0)
        .smd("A1", Some("A"), at(5.0, 5.0), 1.0, 0)
        .smd("A2", Some("A"), at(15.0, 5.0), 1.0, 0)
        .keepout(
            wall(at(8.0, 1.0), at(12.0, 10.0)),
            Some(0),
            KeepoutScope::Everything,
        )
        .build();
    let result = route(&design, &[("A1", "A2")]);
    assert!(result.unrouted.is_empty());
    assert_eq!(result.routes.vias, []);
    assert_legal(&design, &result.routes);
}

#[test]
fn reports_a_connection_it_cannot_route() {
    let design = DesignBuilder::new(20.0, 10.0)
        .smd("A1", Some("A"), at(5.0, 5.0), 1.0, 0)
        .smd("A2", Some("A"), at(15.0, 5.0), 1.0, 0)
        .keepout(
            wall(at(9.5, 0.0), at(10.5, 10.0)),
            None,
            KeepoutScope::Everything,
        )
        .build();
    let result = route(&design, &[("A1", "A2")]);
    assert_eq!(result.unrouted, [connection(&design, "A1", "A2")]);
    assert_eq!(result.routes, Routes::default());
}

#[test]
fn later_connections_keep_clear_of_earlier_ones() {
    let design = DesignBuilder::new(20.0, 20.0)
        .smd("A1", Some("A"), at(2.0, 10.0), 1.0, 0)
        .smd("A2", Some("A"), at(18.0, 10.0), 1.0, 0)
        .smd("B1", Some("B"), at(10.0, 2.0), 1.0, 0)
        .smd("B2", Some("B"), at(10.0, 18.0), 1.0, 0)
        .build();
    let result = route(&design, &[("A1", "A2"), ("B1", "B2")]);
    assert!(result.unrouted.is_empty());
    assert!(
        !result.routes.vias.is_empty(),
        "the crossing needs the other layer"
    );
    assert_legal(&design, &result.routes);
}

#[test]
fn connections_that_may_cross_are_searched_one_after_the_other() {
    let design = DesignBuilder::new(20.0, 20.0)
        .smd("A1", Some("A"), at(2.0, 10.0), 1.0, 0)
        .smd("A2", Some("A"), at(18.0, 10.0), 1.0, 0)
        .smd("B1", Some("B"), at(10.0, 2.0), 1.0, 0)
        .smd("B2", Some("B"), at(10.0, 18.0), 1.0, 0)
        .build();
    let in_one_batch = Profile {
        batch_size: 2,
        ..profile()
    };
    let result = route_on_threads(
        &design,
        &in_one_batch,
        &[("A1", "A2"), ("B1", "B2")],
        None,
        2,
    );
    assert!(result.unrouted.is_empty());
    assert!(
        !result.routes.vias.is_empty(),
        "the crossing needs the other layer"
    );
    assert_legal(&design, &result.routes);
}

/// Twelve two-pin nets with their pads scattered over a 2 mm lattice, so that
/// many of them cross and collide.
fn scattered_nets() -> (Design, Vec<(String, String)>) {
    let mut builder = DesignBuilder::new(22.0, 22.0);
    let mut sites: Vec<(i32, i32)> = (0..10).flat_map(|x| (0..10).map(move |y| (x, y))).collect();
    let mut seed = 7_u64;
    let mut pairs = Vec::new();
    for net in 0..12 {
        let mut ends = Vec::new();
        for end in 1..=2 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let (x, y) = sites.swap_remove((seed >> 33) as usize % sites.len());
            let name = format!("N{net}.{end}");
            builder = builder.smd(
                &name,
                Some(&format!("N{net}")),
                at(2.0 + 2.0 * f64::from(x), 2.0 + 2.0 * f64::from(y)),
                0.8,
                0,
            );
            ends.push(name);
        }
        pairs.push((ends[0].clone(), ends[1].clone()));
    }
    (builder.build(), pairs)
}

#[test]
fn a_board_routed_again_with_layer_directions_stays_complete_and_legal() {
    let (design, pairs) = scattered_nets();
    let pairs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let directed = Profile {
        against_layer_direction: 0.5,
        ..profile()
    };
    let result = route_with(&design, &directed, &pairs);
    assert!(result.unrouted.is_empty(), "{:?}", result.unrouted);
    assert_legal(&design, &result.routes);
}

#[test]
fn the_routes_do_not_depend_on_the_number_of_threads() {
    let (design, pairs) = scattered_nets();
    let pairs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let batched = Profile {
        batch_size: 8,
        heuristic_weight: 1.2,
        ..profile()
    };
    let on_one = route_on_threads(&design, &batched, &pairs, None, 1);
    assert!(on_one.unrouted.is_empty());
    assert_legal(&design, &on_one.routes);
    for threads in [2, 4] {
        let on_more = route_on_threads(&design, &batched, &pairs, None, threads);
        assert_eq!(on_more.routes, on_one.routes, "on {threads} threads");
    }
}

#[test]
fn reaches_pads_that_sit_off_the_grid() {
    let design = DesignBuilder::new(20.0, 10.0)
        .smd("A1", Some("A"), at(5.03, 5.07), 1.0, 0)
        .smd("A2", Some("A"), at(15.01, 3.33), 1.0, 0)
        .build();
    let routes = route(&design, &[("A1", "A2")]).routes;
    let path = &routes.tracks[0].path;
    assert_eq!(
        (path[0], path[path.len() - 1]),
        (at(5.03, 5.07), at(15.01, 3.33))
    );
    assert_legal(&design, &routes);
}

#[test]
fn pads_in_sight_of_each_other_are_joined_at_45_degrees_not_at_any_angle() {
    let design = DesignBuilder::new(20.0, 12.0)
        .smd("A1", Some("A"), at(5.0, 5.0), 1.0, 0)
        .smd("A2", Some("A"), at(15.0, 8.0), 1.0, 0)
        .build();
    let routes = route(&design, &[("A1", "A2")]).routes;
    let path = &routes.tracks[0].path;
    assert_eq!(path.len(), 3, "one bend: {path:?}");
    assert!(
        path.windows(2).all(|p| is_octilinear(p[0], p[1])),
        "{path:?}"
    );
    assert_legal(&design, &routes);
}

#[test]
fn takes_the_other_bend_when_the_first_is_blocked() {
    // A pad of net B sits where the diagonal-first bend would pass.
    let design = DesignBuilder::new(20.0, 12.0)
        .smd("A1", Some("A"), at(5.0, 5.0), 1.0, 0)
        .smd("A2", Some("A"), at(15.0, 8.0), 1.0, 0)
        .smd("B1", Some("B"), at(8.0, 8.0), 1.0, 0)
        .without_vias()
        .build();
    let path = &route(&design, &[("A1", "A2")]).routes.tracks[0].path;
    assert_eq!(path, &[at(5.0, 5.0), at(12.0, 5.0), at(15.0, 8.0)]);
}

/// One layer, a wall down the middle with two gaps one track wide: a lower gap
/// at the board's bottom edge and a middle gap. The short connection Y, routed
/// first, could use either and takes the lower one; the long connection X
/// along the bottom edge can only reach the lower gap.
fn two_gaps() -> Design {
    DesignBuilder::new(20.0, 6.0)
        .layers(1)
        .without_vias()
        .keepout(
            wall(at(9.5, 0.0), at(10.5, 2.75)),
            None,
            KeepoutScope::Everything,
        )
        .keepout(
            wall(at(9.5, 3.25), at(10.5, 5.3)),
            None,
            KeepoutScope::Everything,
        )
        .smd("Y1", Some("Y"), at(8.0, 4.6), 0.4, 0)
        .smd("Y2", Some("Y"), at(12.0, 4.6), 0.4, 0)
        .smd("X1", Some("X"), at(2.0, 5.55), 0.4, 0)
        .smd("X2", Some("X"), at(18.0, 5.55), 0.4, 0)
        .build()
}

fn short_detours() -> Profile {
    Profile {
        detour_margin: 2_000_000,
        ..profile()
    }
}

#[test]
fn without_rip_up_a_connection_that_takes_anothers_only_way_leaves_it_unrouted() {
    let design = one_way_for_two();
    let result = route_with(
        &design,
        &Profile {
            max_rip_ups: 0,
            history_cost: 0,
            ..profile()
        },
        &[("X1", "X2"), ("Y1", "Y2")],
    );
    assert_eq!(result.unrouted, [connection(&design, "Y1", "Y2")]);
}

#[test]
fn rips_up_a_connection_in_the_way_and_routes_it_again_elsewhere() {
    let design = two_gaps();
    let result = route_with(&design, &short_detours(), &[("Y1", "Y2"), ("X1", "X2")]);
    assert_eq!(result.unrouted, []);
    assert_legal(&design, &result.routes);
}

#[test]
fn says_which_connection_ripped_up_which_and_where_they_met() {
    // Y takes the top gap first; X can reach no other and rips it up there.
    let design = two_gaps();
    let rules = traxide_rules::compile(
        &design,
        Purpose::Routing {
            grid_pitch: 100_000,
        },
    );
    let mut layout = Layout::build(&design, &rules);
    let plan = Plan {
        connections: vec![
            connection(&design, "Y1", "Y2"),
            connection(&design, "X1", "X2"),
        ],
    };
    let rip_ups = std::cell::RefCell::new(Vec::new());
    let collect = |event: Event| {
        if let Event::RippedUp { connection, by, at } = event {
            rip_ups.borrow_mut().push((connection, by, at));
        }
    };
    Router::new(&design, &rules, &short_detours()).route_plan(
        &plan,
        None,
        &mut layout,
        &CancelToken::new(),
        &collect,
    );
    let rip_ups = rip_ups.into_inner();
    let &(ripped, by, at) = rip_ups.first().expect("a rip-up");
    assert_eq!((ripped, by), (ConnectionId(0), ConnectionId(1)));
    assert!(
        (mm(8.0)..=mm(12.0)).contains(&at.x) && at.y >= mm(4.0),
        "they meet around the top gap, not at {at:?}"
    );
}

#[test]
fn past_its_deadline_routing_rips_nothing_up_but_routes_what_still_fits() {
    // X and Y need the same only way; before the deadline they would take it
    // from each other in turn.
    let design = one_way_for_two();
    let rules = traxide_rules::compile(
        &design,
        Purpose::Routing {
            grid_pitch: 100_000,
        },
    );
    let mut layout = Layout::build(&design, &rules);
    let plan = Plan {
        connections: vec![
            connection(&design, "X1", "X2"),
            connection(&design, "Y1", "Y2"),
        ],
    };
    let ripped_up = std::cell::Cell::new(false);
    let watch = |event: Event| {
        if matches!(event, Event::RippedUp { .. }) {
            ripped_up.set(true);
        }
    };
    let result = Router::new(&design, &rules, &profile())
        .with_deadline(std::time::Instant::now())
        .route_plan(&plan, None, &mut layout, &CancelToken::new(), &watch);
    assert!(!ripped_up.get());
    assert_eq!(result.unrouted, [connection(&design, "Y1", "Y2")]);
    assert_legal(&design, &result.routes);
}

#[test]
fn a_connection_whose_only_way_runs_further_round_than_its_window_routes() {
    // The wall's end lies 15 mm beyond the pads, three times the margin.
    let design = DesignBuilder::new(40.0, 20.0)
        .layers(1)
        .without_vias()
        .keepout(
            wall(at(19.5, 0.0), at(20.5, 17.0)),
            None,
            KeepoutScope::Everything,
        )
        .smd("A1", Some("A"), at(15.0, 2.0), 0.6, 0)
        .smd("A2", Some("A"), at(25.0, 2.0), 0.6, 0)
        .build();
    let result = route(&design, &[("A1", "A2")]);
    assert_eq!(result.unrouted, []);
    assert_legal(&design, &result.routes);
}

fn one_way_for_two() -> Design {
    DesignBuilder::new(20.0, 6.0)
        .layers(1)
        .without_vias()
        .keepout(
            wall(at(9.5, 0.0), at(10.5, 5.3)),
            None,
            KeepoutScope::Everything,
        )
        .smd("Y1", Some("Y"), at(2.0, 5.55), 0.4, 0)
        .smd("Y2", Some("Y"), at(18.0, 5.55), 0.4, 0)
        .smd("X1", Some("X"), at(3.0, 5.55), 0.4, 0)
        .smd("X2", Some("X"), at(17.0, 5.55), 0.4, 0)
        .build()
}

#[test]
fn gives_up_when_two_connections_need_the_same_only_way() {
    let design = one_way_for_two();
    let result = route(&design, &[("X1", "X2"), ("Y1", "Y2")]);
    assert_eq!(result.unrouted.len(), 1);
    assert_legal(&design, &result.routes);
}

#[test]
fn stops_ripping_up_once_rounds_of_rip_ups_route_no_more_connections() {
    let design = one_way_for_two();
    let rules = traxide_rules::compile(
        &design,
        Purpose::Routing {
            grid_pitch: 100_000,
        },
    );
    let mut layout = Layout::build(&design, &rules);
    let plan = Plan {
        connections: vec![
            connection(&design, "X1", "X2"),
            connection(&design, "Y1", "Y2"),
        ],
    };
    let rip_ups = std::cell::Cell::new(0);
    let count = |event: Event| {
        if matches!(event, Event::RippedUp { .. }) {
            rip_ups.set(rip_ups.get() + 1);
        }
    };
    let result = Router::new(&design, &rules, &profile()).route_plan(
        &plan,
        None,
        &mut layout,
        &CancelToken::new(),
        &count,
    );
    assert_eq!(result.unrouted.len(), 1);
    // Each taking the gap from the other until both were ripped up ten times
    // took 20 rip-ups; two rounds of two rip-ups without gain end at 4.
    assert!(rip_ups.get() <= 4, "{} rip-ups", rip_ups.get());
}

#[test]
fn a_connection_searched_from_its_tighter_end_still_runs_from_its_first_pad() {
    // The small pad offers fewer ways in than the large through-hole pad, so the
    // search starts there; the routes must still read from A1 to A2.
    let design = DesignBuilder::new(20.0, 10.0)
        .tht("A1", Some("A"), at(4.0, 5.0), 2.2)
        .smd("A2", Some("A"), at(15.0, 5.0), 0.3, 0)
        .keepout(
            wall(at(9.5, 0.0), at(10.5, 10.0)),
            Some(0),
            KeepoutScope::Everything,
        )
        .build();
    let routes = route(&design, &[("A1", "A2")]).routes;
    assert_eq!(
        routes.vias.len(),
        1,
        "A1 reaches the bottom layer itself; one via back up"
    );
    let starts: Vec<Point> = routes.tracks.iter().map(|t| t.path[0]).collect();
    let ends: Vec<Point> = routes
        .tracks
        .iter()
        .map(|t| *t.path.last().expect("a path"))
        .collect();
    assert_eq!(starts[0], at(4.0, 5.0));
    assert_eq!(ends[ends.len() - 1], at(15.0, 5.0));
    assert_eq!(
        routes.vias[0].position, ends[0],
        "the via joins the two tracks"
    );
    assert_legal(&design, &routes);
}

#[test]
fn keeps_inside_its_corridor() {
    let design = DesignBuilder::new(20.0, 12.0)
        .smd("A1", Some("A"), at(2.5, 8.5), 1.0, 0)
        .smd("A2", Some("A"), at(17.5, 8.5), 1.0, 0)
        .build();
    // Up from A1, along row 3, down to A2: a detour the straight line would not take.
    let detour = corridor(
        (1..=4)
            .flat_map(|c| (2..=9).map(move |r| (c, r)))
            .chain((1..=18).flat_map(|c| (2..=4).map(move |r| (c, r))))
            .chain((16..=18).flat_map(|c| (2..=9).map(move |r| (c, r)))),
    );
    let result = route_in(&design, &profile(), &[("A1", "A2")], Some(&detour));
    assert_eq!(result.unrouted, []);
    for point in result.routes.tracks.iter().flat_map(|t| &t.path) {
        assert!(
            detour.allows(ConnectionId(0), *point),
            "{point:?} leaves the corridor"
        );
    }
    assert_legal(&design, &result.routes);
}

#[test]
fn searches_the_whole_window_when_its_corridor_is_blocked() {
    let design = DesignBuilder::new(20.0, 12.0)
        .smd("A1", Some("A"), at(2.5, 6.5), 1.0, 0)
        .smd("A2", Some("A"), at(17.5, 6.5), 1.0, 0)
        .keepout(
            wall(at(9.5, 3.0), at(10.5, 12.0)),
            None,
            KeepoutScope::Everything,
        )
        .build();
    // Straight across, right through the wall; the way around its top end lies outside.
    let straight = corridor((1..=18).flat_map(|c| (5..=7).map(move |r| (c, r))));
    let result = route_in(&design, &profile(), &[("A1", "A2")], Some(&straight));
    assert_eq!(result.unrouted, []);
    assert_legal(&design, &result.routes);
}

#[test]
fn a_pad_with_no_way_out_is_named() {
    // A1 lies under a keepout: no track may leave it.
    let design = DesignBuilder::new(20.0, 10.0)
        .keepout(
            wall(at(1.0, 4.0), at(3.0, 6.0)),
            None,
            KeepoutScope::Everything,
        )
        .smd("A1", Some("A"), at(2.0, 5.0), 0.5, 0)
        .smd("A2", Some("A"), at(18.0, 5.0), 0.5, 0)
        .build();
    let result = route(&design, &[("A1", "A2")]);
    assert_eq!(result.unrouted, [connection(&design, "A1", "A2")]);
    assert_eq!(result.hemmed_in, [design.pad_named("A1")]);
}

#[test]
fn a_connection_with_both_pads_hemmed_in_names_both() {
    let design = DesignBuilder::new(20.0, 10.0)
        .keepout(
            wall(at(1.0, 4.0), at(3.0, 6.0)),
            None,
            KeepoutScope::Everything,
        )
        .keepout(
            wall(at(17.0, 4.0), at(19.0, 6.0)),
            None,
            KeepoutScope::Everything,
        )
        .smd("A1", Some("A"), at(2.0, 5.0), 0.5, 0)
        .smd("A2", Some("A"), at(18.0, 5.0), 0.5, 0)
        .build();
    let result = route(&design, &[("A1", "A2")]);
    assert_eq!(
        result.hemmed_in,
        [design.pad_named("A1"), design.pad_named("A2")]
    );
}

#[test]
fn a_pad_whose_ways_out_routed_copper_took_is_not_hemmed_in() {
    // B1, small and off the grid, sits in a pocket of keepouts that leaves it
    // only the way up along its axis; A's track runs across the pocket's mouth,
    // as close as the clearance allows, before B is routed, with no rip-up.
    let pocket = |min, max| (wall(min, max), None, KeepoutScope::Everything);
    let mut builder = DesignBuilder::new(20.0, 10.0).layers(1);
    for (shape, layer, scope) in [
        pocket(at(9.0, 4.0), at(9.91, 5.15)),
        pocket(at(10.19, 4.0), at(11.0, 5.15)),
        pocket(at(9.0, 4.0), at(11.0, 4.91)),
    ] {
        builder = builder.keepout(shape, layer, scope);
    }
    let design = builder
        .smd("B1", Some("B"), at(10.05, 5.05), 0.2, 0)
        .smd("B2", Some("B"), at(10.0, 9.0), 0.5, 0)
        .smd("A1", Some("A"), at(2.0, 5.5), 0.5, 0)
        .smd("A2", Some("A"), at(18.0, 5.5), 0.5, 0)
        .build();
    let no_rip_up = Profile {
        max_rip_ups: 0,
        ..profile()
    };
    assert_eq!(
        route_with(&design, &no_rip_up, &[("B1", "B2")]).unrouted,
        []
    );
    let result = route_with(&design, &no_rip_up, &[("A1", "A2"), ("B1", "B2")]);
    assert_eq!(result.unrouted, [connection(&design, "B1", "B2")]);
    assert_eq!(result.hemmed_in, []);
    assert_eq!(result.exits_taken, [design.pad_named("B1")]);
}

#[test]
fn a_connection_that_fixed_copper_cuts_off_is_named() {
    // B1 sits inside a closed ring of keepout with room around it, B2 outside;
    // one layer, so no via can hop over the ring.
    let ring = [
        wall(at(7.0, 2.0), at(13.0, 2.5)),
        wall(at(7.0, 7.5), at(13.0, 8.0)),
        wall(at(7.0, 2.0), at(7.5, 8.0)),
        wall(at(12.5, 2.0), at(13.0, 8.0)),
    ];
    let mut builder = DesignBuilder::new(20.0, 10.0).layers(1);
    for side in ring {
        builder = builder.keepout(side, None, KeepoutScope::Everything);
    }
    let design = builder
        .smd("B1", Some("B"), at(10.0, 5.0), 0.5, 0)
        .smd("B2", Some("B"), at(17.0, 5.0), 0.5, 0)
        .build();
    let result = route(&design, &[("B1", "B2")]);
    assert_eq!(result.hemmed_in, []);
    assert_eq!(result.cut_off, [connection(&design, "B1", "B2")]);
}

#[test]
fn past_its_deadline_routing_searches_no_further_than_each_window() {
    // As above, but routing has no time left for whole-board searches: B
    // stays unrouted without being found cut off.
    let ring = [
        wall(at(7.0, 2.0), at(13.0, 2.5)),
        wall(at(7.0, 7.5), at(13.0, 8.0)),
        wall(at(7.0, 2.0), at(7.5, 8.0)),
        wall(at(12.5, 2.0), at(13.0, 8.0)),
    ];
    let mut builder = DesignBuilder::new(20.0, 10.0).layers(1);
    for side in ring {
        builder = builder.keepout(side, None, KeepoutScope::Everything);
    }
    let design = builder
        .smd("B1", Some("B"), at(10.0, 5.0), 0.5, 0)
        .smd("B2", Some("B"), at(17.0, 5.0), 0.5, 0)
        .build();
    let rules = traxide_rules::compile(
        &design,
        Purpose::Routing {
            grid_pitch: 100_000,
        },
    );
    let mut layout = Layout::build(&design, &rules);
    let plan = Plan {
        connections: vec![connection(&design, "B1", "B2")],
    };
    let result = Router::new(&design, &rules, &profile())
        .with_deadline(std::time::Instant::now())
        .route_plan(&plan, None, &mut layout, &CancelToken::new(), &Silent);
    assert_eq!(result.unrouted, [connection(&design, "B1", "B2")]);
    assert_eq!(result.cut_off, []);
}

#[test]
fn a_connection_only_routed_copper_cuts_off_is_not_named_cut_off() {
    // A's track crosses the one-layer board from edge to edge before B is
    // routed, with no rip-up: B1 lies below it, B2 above.
    let design = DesignBuilder::new(20.0, 10.0)
        .layers(1)
        .smd("A1", Some("A"), at(0.5, 5.0), 0.5, 0)
        .smd("A2", Some("A"), at(19.5, 5.0), 0.5, 0)
        .smd("B1", Some("B"), at(10.0, 2.0), 0.5, 0)
        .smd("B2", Some("B"), at(10.0, 8.0), 0.5, 0)
        .build();
    let no_rip_up = Profile {
        max_rip_ups: 0,
        ..profile()
    };
    let result = route_with(&design, &no_rip_up, &[("A1", "A2"), ("B1", "B2")]);
    assert_eq!(result.unrouted, [connection(&design, "B1", "B2")]);
    assert_eq!(result.cut_off, []);
}

#[test]
fn a_pad_over_the_board_edge_is_left_from_its_copper_on_the_board() {
    // A1's centre lies on the board's left edge, as an edge connector's might.
    let design = DesignBuilder::new(20.0, 10.0)
        .smd("A1", Some("A"), at(0.0, 5.0), 1.0, 0)
        .smd("A2", Some("A"), at(15.0, 5.0), 1.0, 0)
        .build();
    let result = route(&design, &[("A1", "A2")]);
    assert_eq!(result.unrouted, []);
    assert_legal(&design, &result.routes);
    let start = result.routes.tracks[0].path[0];
    assert!(
        start.x > 0 && start.x <= mm(0.5),
        "starts on A1's copper: {start:?}"
    );
}

/// A row of five 0.2 mm pins at 0.4 mm pitch, as on a QFN, with 0.2 mm track and
/// clearance: a track fits between two neighbours only along a pin's axis.
/// The pins' centres lie `offset_mm` off the grid; each pin connects to a pad on
/// the other layer below the row, fanned out.
fn fine_pitch_row(offset_mm: f64) -> (Design, Vec<(String, String)>) {
    let mut builder = DesignBuilder::new(20.0, 12.0);
    let mut pairs = Vec::new();
    for pin in 0..5 {
        let net = format!("P{pin}");
        let (name, target) = (format!("U1.{pin}"), format!("T{pin}"));
        builder = builder
            .smd(
                &name,
                Some(&net),
                at(9.2 + offset_mm + 0.4 * f64::from(pin), 5.0 + offset_mm),
                0.2,
                0,
            )
            .smd(
                &target,
                Some(&net),
                at(6.0 + 2.0 * f64::from(pin), 9.0),
                0.5,
                1,
            );
        pairs.push((name, target));
    }
    (builder.build(), pairs)
}

fn assert_each_pin_connects_outward(offset_mm: f64) {
    let (design, pairs) = fine_pitch_row(offset_mm);
    let pairs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let result = route(&design, &pairs);
    assert_eq!(result.hemmed_in, []);
    assert_eq!(result.unrouted, []);
    assert_legal(&design, &result.routes);
}

#[test]
fn pins_of_a_fine_pitch_row_half_a_pitch_off_the_grid_leave_along_their_axis() {
    assert_each_pin_connects_outward(0.05);
}

#[test]
fn pins_of_a_fine_pitch_row_a_hair_off_the_grid_leave_along_their_axis() {
    assert_each_pin_connects_outward(0.001);
}

#[test]
fn a_pin_off_the_grid_is_left_by_an_octilinear_stub_along_its_axis() {
    let (design, pairs) = fine_pitch_row(0.05);
    let (pin, target) = (&pairs[2].0, &pairs[2].1);
    let routes = route(&design, &[(pin, target)]).routes;
    let centre = design.pad(design.pad_named(pin)).position;
    let track = routes
        .tracks
        .iter()
        .find(|t| t.path.first() == Some(&centre) || t.path.last() == Some(&centre))
        .expect("a track leaves the pin's centre");
    let mut path = track.path.clone();
    if path[0] != centre {
        path.reverse();
    }
    assert_eq!(path[1].x, centre.x, "first along the axis: {path:?}");
    assert!(
        path.windows(2).all(|p| is_octilinear(p[0], p[1])),
        "{path:?}"
    );
    assert_legal(&design, &routes);
}

#[test]
fn a_route_between_two_pins_off_the_grid_leaves_no_stub_at_either() {
    // Both ends are reached along their pins' axes, by a stub with a jog onto
    // the grid; the one the search finishes at must run from the grid into
    // the pin, not into the pin and back out.
    let mut builder = DesignBuilder::new(20.0, 12.0);
    for (row, y) in [("U1", 3.05), ("U2", 9.05)] {
        for pin in 0..5 {
            let net = if pin == 2 {
                "A".to_owned()
            } else {
                format!("{row}P{pin}")
            };
            builder = builder.smd(
                &format!("{row}.{pin}"),
                Some(&net),
                at(9.25 + 0.4 * f64::from(pin), y),
                0.2,
                0,
            );
        }
    }
    // A wall between the rows, so that no straight line joins them.
    let design = builder
        .keepout(
            wall(at(8.0, 5.5), at(12.0, 6.5)),
            None,
            KeepoutScope::Everything,
        )
        .build();
    let result = route(&design, &[("U1.2", "U2.2")]);
    assert!(result.unrouted.is_empty());
    assert_legal(&design, &result.routes);
}

/// Ten nets on one layer without vias, pads scattered over a 10 × 10 grid at
/// 1 mm: rip-up and reroute leaves one connection unrouted.
fn crowded() -> (Design, Vec<(String, String)>) {
    let mut builder = DesignBuilder::new(12.0, 12.0).layers(1).without_vias();
    let mut sites: Vec<(i32, i32)> = (0..10).flat_map(|x| (0..10).map(move |y| (x, y))).collect();
    let mut seed = 128_u64;
    let mut pairs = Vec::new();
    for net in 0..10 {
        let mut ends = Vec::new();
        for end in 1..=2 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let (x, y) = sites.swap_remove((seed >> 33) as usize % sites.len());
            let name = format!("N{net}.{end}");
            builder = builder.smd(
                &name,
                Some(&format!("N{net}")),
                at(1.5 + f64::from(x), 1.5 + f64::from(y)),
                0.5,
                0,
            );
            ends.push(name);
        }
        pairs.push((ends[0].clone(), ends[1].clone()));
    }
    (builder.build(), pairs)
}

/// Routes `pairs` and returns the result with the layout as routing left it.
fn route_keeping_layout<'d>(
    design: &'d Design,
    rules: &'d traxide_contracts::ConstraintSet,
    pairs: &[(&str, &str)],
) -> (RoutingResult, Layout<'d>) {
    let mut layout = Layout::build(design, rules);
    let plan = Plan {
        connections: pairs
            .iter()
            .map(|&(from, to)| connection(design, from, to))
            .collect(),
    };
    let result = Router::new(design, rules, &profile()).route_plan(
        &plan,
        None,
        &mut layout,
        &CancelToken::new(),
        &Silent,
    );
    (result, layout)
}

/// Whether two layouts hold copper at the same places, probed with a short
/// track of `net` every quarter millimetre.
fn same_copper(design: &Design, a: &Layout, b: &Layout, net: NetId) -> bool {
    let bounds = design.outline.bounds();
    let step = 250_000;
    (bounds.min.x..bounds.max.x).step_by(step).all(|x| {
        (bounds.min.y..bounds.max.y).step_by(step).all(|y| {
            let probe = Segment::new(Point::new(x, y), Point::new(x + 100_000, y));
            let layer = LayerId::from_index(0);
            let holds = |layout: &Layout| {
                layout.track_obstruction(net, layer, probe, 100_000) != Obstruction::None
            };
            holds(a) == holds(b)
        })
    })
}

#[test]
fn a_board_rip_up_leaves_incomplete_is_routed_again_in_rounds_and_the_better_kept() {
    let (design, pairs) = crowded();
    let pairs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let result = route(&design, &pairs);
    assert_eq!(result.unrouted, []);
    assert_legal(&design, &result.routes);
}

#[test]
fn the_layout_holds_the_copper_of_the_attempt_kept_and_no_other() {
    let (crowded, pairs) = crowded();
    let crowded_pairs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let one_way = one_way_for_two();
    // On one_way_for_two both attempts route one connection and the first is
    // kept; on the crowded board the second routes more.
    for (design, pairs) in [
        (&one_way, vec![("X1", "X2"), ("Y1", "Y2")]),
        (&crowded, crowded_pairs),
    ] {
        let rules = traxide_rules::compile(
            design,
            Purpose::Routing {
                grid_pitch: 100_000,
            },
        );
        let (result, layout) = route_keeping_layout(design, &rules, &pairs);
        let mut kept = Layout::build(design, &rules);
        kept.add_route(ConnectionId::from_index(0), &result.routes);
        // A probe meets the copper of every net but its own: two nets see all.
        for (pad, _) in &pairs[..2] {
            let net = design.pad(design.pad_named(pad)).net.expect("a net");
            assert!(same_copper(design, &layout, &kept, net), "{pad}");
        }
    }
}

#[test]
fn a_hop_under_a_track_that_could_run_on_the_other_layer_is_taken_out() {
    // B, routed first, runs straight on the first layer; the walls leave A no
    // way round B's ends, so A hops under it. B's through-hole pads let it run
    // on the second layer as well, where A no longer needs a via at all.
    let design = DesignBuilder::new(20.0, 20.0)
        .layers(2)
        .smd("A1", Some("A"), at(2.0, 10.0), 1.0, 0)
        .smd("A2", Some("A"), at(18.0, 10.0), 1.0, 0)
        .tht("B1", Some("B"), at(10.0, 3.0), 1.2)
        .tht("B2", Some("B"), at(10.0, 17.0), 1.2)
        .keepout(
            wall(at(9.0, 0.0), at(11.0, 2.0)),
            None,
            KeepoutScope::Everything,
        )
        .keepout(
            wall(at(9.0, 18.0), at(11.0, 20.0)),
            None,
            KeepoutScope::Everything,
        )
        .build();
    let result = route(&design, &[("B1", "B2"), ("A1", "A2")]);
    assert!(result.unrouted.is_empty());
    assert_eq!(result.routes.vias, [], "{:?}", result.routes.tracks);
    assert_legal(&design, &result.routes);
}

fn track_length(routes: &Routes) -> f64 {
    routes
        .tracks
        .iter()
        .flat_map(|t| t.path.windows(2))
        .map(|w| f64::from(w[1].x - w[0].x).hypot(f64::from(w[1].y - w[0].y)))
        .sum()
}

#[test]
fn a_pad_beside_its_nets_track_joins_the_track_rather_than_the_far_pad() {
    // P3 is planned to P1, but P1's track to P2 passes 5 mm below P3: the
    // net grows as a tree, P3 dropping straight onto the track.
    let design = DesignBuilder::new(20.0, 20.0)
        .smd("P1", Some("A"), at(2.0, 10.0), 1.0, 0)
        .smd("P2", Some("A"), at(18.0, 10.0), 1.0, 0)
        .smd("P3", Some("A"), at(10.0, 15.0), 1.0, 0)
        .build();
    let result = route(&design, &[("P1", "P2"), ("P3", "P1")]);
    assert!(result.unrouted.is_empty());
    let length = track_length(&result.routes);
    assert!(
        length < mm(21.5) as f64,
        "{length} nm: {:?}",
        result.routes.tracks
    );
    assert_legal(&design, &result.routes);
}

#[test]
fn a_pad_on_its_nets_track_keeps_a_track_of_its_own() {
    // P3 lies on P1's track to P2, which joins it only by lying over it:
    // pulled tight later, that track may move off P3. Growing trees leaves
    // P3's connection some copper that ends at P3.
    let design = DesignBuilder::new(20.0, 20.0)
        .smd("P1", Some("A"), at(2.0, 10.0), 1.0, 0)
        .smd("P2", Some("A"), at(18.0, 10.0), 1.0, 0)
        .smd("P3", Some("A"), at(10.0, 10.0), 1.0, 0)
        .build();
    let result = route(&design, &[("P1", "P2"), ("P3", "P1")]);
    assert!(result.unrouted.is_empty());
    let p3 = at(10.0, 10.0);
    assert!(
        result
            .routes
            .tracks
            .iter()
            .any(|t| t.path.first() == Some(&p3) || t.path.last() == Some(&p3)),
        "{:?}",
        result.routes.tracks
    );
    assert_legal(&design, &result.routes);
}
