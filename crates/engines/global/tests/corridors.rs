use traxide_contracts::geometry::{Point, Polygon, Rect, Shape};
use traxide_contracts::testing::{DesignBuilder, at};
use traxide_contracts::{Connection, ConnectionId, Corridors, Design, KeepoutScope, Plan, Profile};
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
        global_routing: true,
        batch_size: 1,
        against_layer_direction: 0.0,
    }
}

fn plan(design: &Design, pairs: &[(&str, &str)]) -> Plan {
    let connection = |from: &str, to: &str| {
        let from = design.pad_named(from);
        Connection {
            net: design.pad(from).net.expect("a net"),
            from,
            to: design.pad_named(to),
        }
    };
    Plan {
        connections: pairs.iter().map(|&(a, b)| connection(a, b)).collect(),
    }
}

fn corridors(design: &Design, pairs: &[(&str, &str)]) -> Corridors {
    let rules = traxide_rules::compile(
        design,
        Purpose::Routing {
            grid_pitch: 100_000,
        },
    );
    traxide_global::route(design, &plan(design, pairs), &rules, &profile())
}

fn wall(min: Point, max: Point) -> Shape {
    Shape::Polygon(Polygon::rectangle(Rect { min, max }))
}

#[test]
fn a_corridor_runs_along_the_way_between_the_pads() {
    let design = DesignBuilder::new(40.0, 20.0)
        .smd("A1", Some("A"), at(5.0, 10.0), 1.0, 0)
        .smd("A2", Some("A"), at(35.0, 10.0), 1.0, 0)
        .build();
    let corridors = corridors(&design, &[("A1", "A2")]);
    let a = ConnectionId(0);
    for x in [5.0, 12.0, 20.0, 28.0, 35.0] {
        assert!(corridors.allows(a, at(x, 10.0)), "on the way at x = {x}");
    }
    assert!(!corridors.allows(a, at(20.0, 18.0)), "far off the way");
}

#[test]
fn a_corridor_goes_around_what_blocks_the_straight_way() {
    let design = DesignBuilder::new(40.0, 20.0)
        .smd("A1", Some("A"), at(5.0, 10.0), 1.0, 0)
        .smd("A2", Some("A"), at(35.0, 10.0), 1.0, 0)
        .keepout(
            wall(at(19.0, 0.0), at(21.0, 15.0)),
            None,
            KeepoutScope::Everything,
        )
        .build();
    let corridors = corridors(&design, &[("A1", "A2")]);
    assert!(
        corridors.allows(ConnectionId(0), at(20.0, 17.5)),
        "through the gap above the wall's end"
    );
    assert!(
        !corridors.allows(ConnectionId(0), at(20.0, 10.0)),
        "not through the wall"
    );
}

#[test]
fn connections_share_out_a_gap_too_narrow_for_all_of_them() {
    // A gap one lane wide in the middle of the wall, and open space around its ends.
    let design = DesignBuilder::new(40.0, 40.0)
        .layers(1)
        .keepout(
            wall(at(19.0, 5.0), at(21.0, 19.8)),
            None,
            KeepoutScope::Everything,
        )
        .keepout(
            wall(at(19.0, 20.2), at(21.0, 35.0)),
            None,
            KeepoutScope::Everything,
        )
        .smd("A1", Some("A"), at(5.0, 19.0), 1.0, 0)
        .smd("A2", Some("A"), at(35.0, 19.0), 1.0, 0)
        .smd("B1", Some("B"), at(5.0, 21.0), 1.0, 0)
        .smd("B2", Some("B"), at(35.0, 21.0), 1.0, 0)
        .build();
    let corridors = corridors(&design, &[("A1", "A2"), ("B1", "B2")]);
    let through_gap = |c| corridors.allows(ConnectionId(c), at(20.0, 20.0));
    assert!(
        !(through_gap(0) && through_gap(1)),
        "only one of them fits through the gap"
    );
}

#[test]
fn the_same_design_always_gets_the_same_corridors() {
    let design = DesignBuilder::new(40.0, 20.0)
        .smd("A1", Some("A"), at(5.0, 10.0), 1.0, 0)
        .smd("A2", Some("A"), at(35.0, 12.0), 1.0, 0)
        .smd("B1", Some("B"), at(20.0, 2.0), 1.0, 0)
        .smd("B2", Some("B"), at(20.0, 18.0), 1.0, 0)
        .build();
    let pairs = [("A1", "A2"), ("B1", "B2")];
    assert_eq!(corridors(&design, &pairs), corridors(&design, &pairs));
}
