use traxide_contracts::geometry::{Point, Polygon, Rect, Region, Segment, Shape};
use traxide_contracts::testing::{DesignBuilder, at, mm};
use traxide_contracts::{
    ConnectionId, Design, KeepoutScope, LayerId, LayoutAccess, NetId, Obstruction, Routes, Track,
    Via, ViaKindId,
};
use traxide_layout_access::Layout;

const TOP: LayerId = LayerId(0);
const BOTTOM: LayerId = LayerId(1);

/// Two 1 mm pads of nets A and B, side by side in the middle of a 20 × 10 mm board.
fn board() -> Design {
    DesignBuilder::new(20.0, 10.0)
        .smd("P1", Some("A"), at(5.0, 5.0), 1.0, 0)
        .smd("P2", Some("B"), at(15.0, 5.0), 1.0, 0)
        .build()
}

fn segment(from: Point, to: Point) -> Segment {
    Segment::new(from, to)
}

fn track_of(net: NetId, along: Segment) -> Routes {
    Routes {
        tracks: vec![Track {
            net,
            layer: TOP,
            width: mm(0.2),
            path: vec![along.a, along.b],
        }],
        vias: vec![],
    }
}

fn check(design: &Design, f: impl FnOnce(&mut Layout<'_>, NetId, NetId)) {
    let rules = traxide_rules::compile(
        design,
        traxide_rules::Purpose::Routing {
            grid_pitch: 100_000,
        },
    );
    let mut layout = Layout::build(design, &rules);
    f(&mut layout, design.net_named("A"), design.net_named("B"));
}

#[test]
fn an_empty_stretch_of_board_is_clear() {
    check(&board(), |layout, a, _| {
        assert!(layout.is_track_clear(a, TOP, segment(at(8.0, 2.0), at(12.0, 2.0)), mm(0.2)));
    });
}

#[test]
fn a_track_may_run_into_a_pad_of_its_own_net() {
    check(&board(), |layout, a, _| {
        assert!(layout.is_track_clear(a, TOP, segment(at(5.0, 5.0), at(10.0, 5.0)), mm(0.2)));
    });
}

#[test]
fn a_track_must_not_touch_a_pad_of_another_net() {
    check(&board(), |layout, a, _| {
        assert!(!layout.is_track_clear(a, TOP, segment(at(10.0, 5.0), at(15.0, 5.0)), mm(0.2)));
    });
}

#[test]
fn a_track_keeps_exactly_the_clearance_from_another_net() {
    // The pad of B ends at x = 14.5 mm; a 0.2 mm track ends 0.1 mm past its centre line.
    check(&board(), |layout, a, _| {
        let up_to = |x: i32| segment(at(10.0, 5.0), Point::new(x, mm(5.0)));
        assert!(layout.is_track_clear(a, TOP, up_to(mm(14.5 - 0.2 - 0.1)), mm(0.2)));
        assert!(!layout.is_track_clear(a, TOP, up_to(mm(14.5 - 0.2 - 0.1) + 1), mm(0.2)));
    });
}

#[test]
fn layers_do_not_interfere() {
    check(&board(), |layout, a, _| {
        assert!(layout.is_track_clear(a, BOTTOM, segment(at(10.0, 5.0), at(15.0, 5.0)), mm(0.2)));
    });
}

#[test]
fn copper_keeps_its_clearance_from_the_board_edge() {
    check(&board(), |layout, a, _| {
        assert!(layout.is_track_clear(a, TOP, segment(at(1.0, 0.3), at(3.0, 0.3)), mm(0.2)));
        assert!(!layout.is_track_clear(a, TOP, segment(at(1.0, 0.29), at(3.0, 0.29)), mm(0.2)));
        assert!(!layout.is_track_clear(a, TOP, segment(at(19.0, 5.0), at(21.0, 5.0)), mm(0.2)));
    });
}

#[test]
fn added_tracks_block_other_nets_only() {
    check(&board(), |layout, a, b| {
        let across = segment(at(10.0, 1.0), at(10.0, 9.0));
        layout.add_route(ConnectionId(0), &track_of(a, across));
        let crossing = segment(at(8.0, 3.0), at(12.0, 3.0));
        assert!(!layout.is_track_clear(b, TOP, crossing, mm(0.2)));
        assert!(layout.is_track_clear(a, TOP, crossing, mm(0.2)));
    });
}

#[test]
fn a_via_needs_room_on_every_layer_it_spans() {
    let design = DesignBuilder::new(20.0, 10.0)
        .smd("P1", Some("A"), at(5.0, 5.0), 1.0, 1)
        .smd("P2", Some("B"), at(18.0, 5.0), 1.0, 1)
        .build();
    let rules = traxide_rules::compile(
        &design,
        traxide_rules::Purpose::Routing {
            grid_pitch: 100_000,
        },
    );
    let mut layout = Layout::build(&design, &rules);
    let other = design.net_named("B");
    let via = |x: f64| Via {
        net: other,
        kind: ViaKindId(0),
        position: at(x, 5.0),
    };
    assert!(
        !layout.is_via_clear(&via(5.9)),
        "the pad on the bottom layer is in the way"
    );
    assert!(layout.is_via_clear(&via(8.0)));
    layout.add_route(
        ConnectionId(0),
        &Routes {
            tracks: vec![],
            vias: vec![via(8.0)],
        },
    );
    assert!(!layout.is_track_clear(
        design.net_named("A"),
        TOP,
        segment(at(7.0, 5.0), at(9.0, 5.0)),
        mm(0.2)
    ));
}

#[test]
fn keepouts_block_what_they_are_meant_to_block() {
    let zone = Shape::Polygon(Polygon::rectangle(Rect {
        min: at(9.0, 0.0),
        max: at(11.0, 10.0),
    }));
    let design = DesignBuilder::new(20.0, 10.0)
        .smd("P1", Some("A"), at(5.0, 5.0), 1.0, 0)
        .keepout(zone, None, KeepoutScope::ViasOnly)
        .build();
    let rules = traxide_rules::compile(
        &design,
        traxide_rules::Purpose::Routing {
            grid_pitch: 100_000,
        },
    );
    let layout = Layout::build(&design, &rules);
    let a = design.net_named("A");
    assert!(layout.is_track_clear(a, TOP, segment(at(8.0, 5.0), at(12.0, 5.0)), mm(0.2)));
    assert!(!layout.is_via_clear(&Via {
        net: a,
        kind: ViaKindId(0),
        position: at(10.0, 5.0)
    }));
}

#[test]
fn obstructions_name_the_routed_connections_in_the_way() {
    let design = DesignBuilder::new(20.0, 10.0)
        .smd("P1", Some("A"), at(2.0, 5.0), 1.0, 0)
        .smd("P2", Some("B"), at(18.0, 5.0), 1.0, 0)
        .smd("P3", Some("C"), at(18.0, 8.0), 1.0, 0)
        .build();
    let rules = traxide_rules::compile(
        &design,
        traxide_rules::Purpose::Routing {
            grid_pitch: 100_000,
        },
    );
    let mut layout = Layout::build(&design, &rules);
    let (a, b, c) = (
        design.net_named("A"),
        design.net_named("B"),
        design.net_named("C"),
    );
    layout.add_route(
        ConnectionId(7),
        &track_of(b, segment(at(10.0, 1.0), at(10.0, 9.0))),
    );
    layout.add_route(
        ConnectionId(3),
        &track_of(c, segment(at(12.0, 1.0), at(12.0, 9.0))),
    );
    let across = segment(at(8.0, 5.0), at(14.0, 5.0));
    assert_eq!(
        layout.track_obstruction(a, TOP, across, mm(0.2)),
        Obstruction::Routed(vec![ConnectionId(3), ConnectionId(7)])
    );
    assert_eq!(
        layout.track_obstruction(a, BOTTOM, across, mm(0.2)),
        Obstruction::None
    );
}

#[test]
fn pads_and_the_board_edge_are_fixed_obstructions() {
    check(&board(), |layout, a, _| {
        let into_b = segment(at(10.0, 5.0), at(15.0, 5.0));
        assert_eq!(
            layout.track_obstruction(a, TOP, into_b, mm(0.2)),
            Obstruction::Fixed
        );
        let off_board = segment(at(19.0, 5.0), at(21.0, 5.0));
        assert_eq!(
            layout.track_obstruction(a, TOP, off_board, mm(0.2)),
            Obstruction::Fixed
        );
    });
}

#[test]
fn removing_a_route_clears_its_copper() {
    check(&board(), |layout, a, b| {
        layout.add_route(
            ConnectionId(0),
            &track_of(a, segment(at(10.0, 1.0), at(10.0, 9.0))),
        );
        let crossing = segment(at(8.0, 3.0), at(12.0, 3.0));
        assert!(!layout.is_track_clear(b, TOP, crossing, mm(0.2)));
        layout.remove_route(ConnectionId(0));
        assert!(layout.is_track_clear(b, TOP, crossing, mm(0.2)));
    });
}

/// A 20 × 10 mm board without its top right quarter, like a board with a notch;
/// its two pads sit far from where the tests look.
fn l_shaped_board() -> Design {
    let mut design = DesignBuilder::new(20.0, 10.0)
        .smd("P1", Some("A"), at(1.0, 9.0), 0.5, 0)
        .smd("P2", Some("B"), at(2.0, 9.0), 0.5, 0)
        .build();
    design.outline = Region::from(
        Polygon::new(vec![
            at(0.0, 0.0),
            at(10.0, 0.0),
            at(10.0, 5.0),
            at(20.0, 5.0),
            at(20.0, 10.0),
            at(0.0, 10.0),
        ])
        .expect("a polygon"),
    );
    design
}

#[test]
fn copper_in_a_cut_out_is_off_the_board() {
    check(&l_shaped_board(), |layout, a, _| {
        assert!(!layout.is_track_clear(a, TOP, segment(at(14.0, 2.0), at(16.0, 2.0)), mm(0.2)));
    });
}

#[test]
fn copper_keeps_its_clearance_from_every_edge_of_a_shaped_board() {
    check(&l_shaped_board(), |layout, a, _| {
        let clear = |from, to| layout.is_track_clear(a, TOP, segment(from, to), mm(0.2));
        assert!(clear(at(12.0, 5.3), at(16.0, 5.3)));
        assert!(!clear(at(12.0, 5.29), at(16.0, 5.29)));
        assert!(clear(at(9.7, 1.0), at(9.7, 4.0)));
        assert!(!clear(at(9.71, 1.0), at(9.71, 4.0)));
        assert!(clear(at(4.0, 7.0), at(8.0, 7.0)));
    });
}

#[test]
fn copper_keeps_the_clearance_a_rule_sets_for_the_footprint_that_draws_a_cutout() {
    use traxide_contracts::{Condition, Constraint, CustomRule, Item, Limits};
    // A cutout of footprint LED1, whose rule allows 0.05 mm, and one of the
    // board's own, at the board's 0.2 mm.
    let mut design = board();
    let cutout = |x: f64| {
        Polygon::rectangle(Rect {
            min: at(x, 6.0),
            max: at(x + 2.0, 8.0),
        })
    };
    design.outline = design
        .outline
        .clone()
        .with_holes(vec![cutout(2.0), cutout(12.0)]);
    design.cutout_footprints = vec![Some("LED1".into()), None];
    design.rules.custom = vec![CustomRule {
        name: "LED cutout".into(),
        condition: Condition::MemberOfFootprint {
            item: Item::B,
            footprints: vec!["LED1".into()],
        },
        constraint: Constraint::EdgeClearance,
        limits: Limits {
            min: Some(mm(0.05)),
            ..Limits::default()
        },
    }];
    check(&design, |layout, a, _| {
        // 0.1 mm below each cutout: the track's edge at 5.9 mm.
        assert!(layout.is_track_clear(a, TOP, segment(at(2.5, 5.8), at(3.5, 5.8)), mm(0.2)));
        assert!(!layout.is_track_clear(a, TOP, segment(at(12.5, 5.8), at(13.5, 5.8)), mm(0.2)));
    });
}

#[test]
fn copper_keeps_a_keepouts_clearance_from_it() {
    let zone = Shape::Polygon(Polygon::rectangle(Rect {
        min: at(9.0, 6.0),
        max: at(11.0, 10.0),
    }));
    let mut design = DesignBuilder::new(20.0, 10.0)
        .smd("P1", Some("A"), at(5.0, 2.0), 1.0, 0)
        .smd("P2", Some("B"), at(15.0, 2.0), 1.0, 0)
        .keepout(zone, None, KeepoutScope::Everything)
        .build();
    // 0.1 mm below the keepout: clear of a rule area, too close to a keepout
    // that asks 0.2 mm.
    let below = segment(at(8.0, 5.8), at(12.0, 5.8));
    check(&design, |layout, a, _| {
        assert!(layout.is_track_clear(a, TOP, below, mm(0.2)));
    });
    design.keepouts[0].clearance = mm(0.2);
    check(&design, |layout, a, _| {
        assert!(!layout.is_track_clear(a, TOP, below, mm(0.2)));
    });
}

#[test]
fn a_via_keeps_the_hole_to_hole_distance_but_may_sit_on_a_via_of_its_net() {
    let mut design = board();
    design.rules.hole_to_hole = mm(0.25);
    check(&design, |layout, a, _| {
        let via = |x| Via {
            net: a,
            kind: ViaKindId(0),
            position: at(x, 5.0),
        };
        layout.add_route(
            ConnectionId(0),
            &Routes {
                tracks: vec![],
                vias: vec![via(10.0)],
            },
        );
        // Drills of 0.3 mm: on the via itself it is the same hole; 0.5 mm
        // apart the holes are 0.2 mm apart, 0.6 mm apart 0.3 mm.
        assert!(layout.is_via_clear(&via(10.0)));
        assert!(!layout.is_via_clear(&via(10.5)));
        assert!(layout.is_via_clear(&via(10.6)));
    });
}

#[test]
fn a_track_overlaps_only_routed_copper_it_runs_over_not_copper_too_close() {
    // B's track runs up the middle of the board. A track across it overlaps
    // it; one alongside, 0.3 mm from its centre, comes closer than the
    // clearance but overlaps nothing.
    check(&board(), |layout, a, b| {
        layout.add_route(
            ConnectionId(4),
            &track_of(b, segment(at(10.0, 1.0), at(10.0, 9.0))),
        );
        let across = segment(at(8.0, 5.0), at(12.0, 5.0));
        let alongside = segment(at(10.3, 2.0), at(10.3, 8.0));
        assert_eq!(
            layout.track_overlaps(a, TOP, across, mm(0.2)),
            [ConnectionId(4)]
        );
        assert_ne!(
            layout.track_obstruction(a, TOP, alongside, mm(0.2)),
            Obstruction::None
        );
        assert_eq!(layout.track_overlaps(a, TOP, alongside, mm(0.2)), []);
        assert_eq!(layout.track_overlaps(b, TOP, across, mm(0.2)), []);
    });
}
