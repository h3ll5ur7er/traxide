use traxide_contracts::geometry::{Point, Polygon, Rect, Segment, Shape};
use traxide_contracts::testing::{DesignBuilder, at, mm};
use traxide_contracts::{
    Connection, CoupledConnections, Design, KeepoutScope, LayerId, NetId, Profile, Routes, Track,
    Violation,
};
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
        rounds: 2,
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

/// A board 20 by 10 mm whose pair D_P, D_N runs from pads 1 mm apart at the
/// left to pads 1 mm apart at the right, 0.2 mm tracks 0.25 mm apart.
fn pair_board() -> DesignBuilder {
    pair_board_with_vias().without_vias()
}

fn pair_board_with_vias() -> DesignBuilder {
    DesignBuilder::new(20.0, 10.0)
        .differential_pairs(0.2, 0.25)
        .smd("P1", Some("D_P"), at(2.0, 4.5), 0.5, 0)
        .smd("N1", Some("D_N"), at(2.0, 5.5), 0.5, 0)
        .smd("P2", Some("D_P"), at(18.0, 4.5), 0.5, 0)
        .smd("N2", Some("D_N"), at(18.0, 5.5), 0.5, 0)
}

fn route_pair(design: &Design) -> Routes {
    let rules = traxide_rules::compile(
        design,
        Purpose::Routing {
            grid_pitch: 100_000,
        },
    );
    let coupled = [CoupledConnections {
        pair: rules.pairs()[0],
        connections: [
            connection(design, "P1", "P2"),
            connection(design, "N1", "N2"),
        ],
    }];
    let mut layout = Layout::build(design, &rules);
    let [routes] = traxide_detailed::route_pairs(design, &rules, &profile(), &coupled, &mut layout)
        .try_into()
        .expect("one for each pair");
    routes.unwrap_or_default()
}

/// No copper of `routes` breaks a rule, though it may leave pads unjoined.
fn assert_clear(design: &Design, routes: &Routes) {
    let rules = traxide_rules::compile(design, Purpose::Verification);
    let verdict = traxide_verification::verify(design, &rules, routes);
    let broken: Vec<_> = (verdict.violations.iter())
        .filter(|v| {
            !matches!(
                v,
                Violation::Unconnected { .. } | Violation::Dangling { .. }
            )
        })
        .collect();
    assert!(broken.is_empty(), "{broken:?}");
}

fn assert_legal(design: &Design, routes: &Routes) {
    let rules = traxide_rules::compile(design, Purpose::Verification);
    let verdict = traxide_verification::verify(design, &rules, routes);
    assert_eq!(verdict.violations, Vec::<Violation>::new());
}

fn tracks_of(routes: &Routes, net: NetId) -> Vec<&Track> {
    routes.tracks.iter().filter(|t| t.net == net).collect()
}

fn distance_to(tracks: &[&Track], layer: LayerId, at: Point) -> i64 {
    (tracks.iter().filter(|t| t.layer == layer))
        .flat_map(|t| t.path.windows(2))
        .map(|pair| at.distance(Segment::new(pair[0], pair[1]).closest_point(at)))
        .min()
        .unwrap_or(i64::MAX)
}

/// The share of `tracks`' length, beyond `fan_in` of the points in `apart`,
/// whose centre runs `pitch` from `others`' on its layer, give or take a
/// micrometre; never closer.
fn coupled_share(
    tracks: &[&Track],
    others: &[&Track],
    pitch: i32,
    apart: &[Point],
    fan_in: i64,
) -> f64 {
    let (mut coupled, mut all) = (0, 0);
    for track in tracks {
        for pair in track.path.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let steps = (a.distance(b) / 10_000).max(1);
            for k in 0..steps {
                let at = Point::new(
                    a.x + ((b.x - a.x) as i64 * (2 * k + 1) / (2 * steps)) as i32,
                    a.y + ((b.y - a.y) as i64 * (2 * k + 1) / (2 * steps)) as i32,
                );
                if apart.iter().any(|&p| at.distance(p) < fan_in) {
                    continue;
                }
                let apart = distance_to(others, track.layer, at);
                assert!(apart >= i64::from(pitch) - 1_000, "{apart} apart at {at:?}");
                all += 1;
                if apart <= i64::from(pitch) + 1_000 {
                    coupled += 1;
                }
            }
        }
    }
    coupled as f64 / all.max(1) as f64
}

fn assert_coupled(design: &Design, routes: &Routes) {
    let positive = tracks_of(routes, design.net_named("D_P"));
    let negative = tracks_of(routes, design.net_named("D_N"));
    let ends = |tracks: &[&Track]| -> Vec<Point> {
        (tracks.iter())
            .flat_map(|t| [t.path[0], *t.path.last().expect("a path")])
            .collect()
    };
    for (tracks, [from, to]) in [
        (&positive, [at(2.0, 4.5), at(18.0, 4.5)]),
        (&negative, [at(2.0, 5.5), at(18.0, 5.5)]),
    ] {
        assert!(
            ends(tracks).contains(&from) && ends(tracks).contains(&to),
            "{tracks:?}"
        );
        assert!(tracks.iter().all(|t| t.width == mm(0.2)));
    }
    let mut apart = vec![at(2.0, 4.5), at(18.0, 4.5), at(2.0, 5.5), at(18.0, 5.5)];
    apart.extend(routes.vias.iter().map(|v| v.position));
    for (tracks, others) in [(&positive, &negative), (&negative, &positive)] {
        let share = coupled_share(tracks, others, mm(0.45), &apart, mm(1.0).into());
        assert!(share > 0.9, "{share} of it coupled");
    }
}

#[test]
fn a_pair_routes_as_two_tracks_at_its_gap() {
    let design = pair_board().build();
    let routes = route_pair(&design);
    assert_coupled(&design, &routes);
    assert_legal(&design, &routes);
}

#[test]
fn a_pair_goes_around_what_is_in_its_way_together() {
    let wall = Shape::Polygon(Polygon::rectangle(Rect {
        min: at(9.0, 2.0),
        max: at(11.0, 8.0),
    }));
    let design = pair_board()
        .keepout(wall, None, KeepoutScope::Everything)
        .build();
    let routes = route_pair(&design);
    assert_coupled(&design, &routes);
    assert!(
        routes.tracks.iter().all(|t| t.path.len() > 4),
        "around the wall"
    );
    assert_legal(&design, &routes);
}

#[test]
fn a_pair_with_room_for_one_track_alone_is_left_to_route_alone() {
    let wall = |min, max| Shape::Polygon(Polygon::rectangle(Rect { min, max }));
    let design = pair_board()
        .keepout(
            wall(at(9.0, 0.0), at(11.0, 4.75)),
            None,
            KeepoutScope::Everything,
        )
        .keepout(
            wall(at(9.0, 5.25), at(11.0, 10.0)),
            None,
            KeepoutScope::Everything,
        )
        .build();
    assert_eq!(route_pair(&design), Routes::default());
}

#[test]
fn a_pair_changes_layers_together_through_two_vias_side_by_side() {
    let wall = Shape::Polygon(Polygon::rectangle(Rect {
        min: at(9.0, 0.0),
        max: at(11.0, 10.0),
    }));
    let design = pair_board_with_vias()
        .keepout(wall, Some(0), KeepoutScope::Everything)
        .build();
    let routes = route_pair(&design);
    assert_eq!(routes.vias.len(), 4, "{routes:?}");
    assert!(routes.tracks.iter().any(|t| t.layer == LayerId(1)));
    assert_coupled(&design, &routes);
    assert_legal(&design, &routes);
}

#[test]
fn a_pair_with_no_room_to_fan_in_at_one_end_stops_short_of_its_pads_there() {
    let design = DesignBuilder::new(18.6, 10.0)
        .differential_pairs(0.2, 0.25)
        .without_vias()
        .smd("P1", Some("D_P"), at(2.0, 4.5), 0.5, 0)
        .smd("N1", Some("D_N"), at(2.0, 5.5), 0.5, 0)
        .smd("P2", Some("D_P"), at(18.0, 4.5), 0.5, 0)
        .smd("N2", Some("D_N"), at(18.0, 5.5), 0.5, 0)
        .smd("X", Some("OTHER"), at(17.3, 5.0), 0.2, 0)
        .build();
    let routes = route_pair(&design);
    let positive = tracks_of(&routes, design.net_named("D_P"));
    let negative = tracks_of(&routes, design.net_named("D_N"));
    let points =
        |tracks: &[&Track]| -> Vec<Point> { tracks.iter().flat_map(|t| t.path.clone()).collect() };
    for (tracks, [from, to]) in [
        (&positive, [at(2.0, 4.5), at(18.0, 4.5)]),
        (&negative, [at(2.0, 5.5), at(18.0, 5.5)]),
    ] {
        assert!(points(tracks).contains(&from), "{tracks:?}");
        assert!(!points(tracks).contains(&to), "{tracks:?}");
        let nearest = points(tracks).into_iter().map(|p| p.distance(to)).min();
        assert!(nearest < Some(mm(2.0).into()), "{tracks:?}");
    }
    let pads = [at(2.0, 4.5), at(2.0, 5.5), at(18.0, 4.5), at(18.0, 5.5)];
    for (tracks, others) in [(&positive, &negative), (&negative, &positive)] {
        let share = coupled_share(tracks, others, mm(0.45), &pads, mm(1.5).into());
        assert!(share > 0.9, "{share} of it coupled");
    }
    assert_clear(&design, &routes);
}

#[test]
fn a_pair_changing_layers_over_a_ground_pour_takes_a_ground_via_beside_its_vias() {
    let wall = Shape::Polygon(Polygon::rectangle(Rect {
        min: at(9.0, 0.0),
        max: at(11.0, 10.0),
    }));
    let design = pair_board_with_vias()
        .keepout(wall, Some(0), KeepoutScope::Everything)
        .plane("GND", 0, at(0.0, 0.0), at(20.0, 10.0))
        .plane("GND", 1, at(0.0, 0.0), at(20.0, 10.0))
        .build();
    let routes = route_pair(&design);
    let gnd = design.net_named("GND");
    let (stitching, pair): (Vec<_>, Vec<_>) = routes.vias.iter().partition(|v| v.net == gnd);
    assert_eq!((pair.len(), stitching.len()), (4, 2), "{routes:?}");
    for via in &stitching {
        let nearest = pair.iter().map(|p| p.position.distance(via.position)).min();
        assert!(
            nearest < Some(mm(1.5).into()),
            "{via:?} far from the pair's vias"
        );
    }
    assert_coupled(&design, &routes);
    assert_legal(&design, &routes);
}
