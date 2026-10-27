use traxide_contracts::geometry::Point;
use traxide_contracts::testing::{DesignBuilder, at, mm};
use traxide_contracts::{
    CopperItem, Design, LayerId, Routes, Track, Verdict, Via, ViaKindId, Violation,
};
use traxide_rules::{Purpose, compile};
use traxide_verification::verify;

/// Net A from (2, 5) to (18, 5) mm, net B's pad in the middle at (10, 8) mm.
fn board() -> Design {
    DesignBuilder::new(20.0, 10.0)
        .smd("A1", Some("A"), at(2.0, 5.0), 1.0, 0)
        .smd("A2", Some("A"), at(18.0, 5.0), 1.0, 0)
        .smd("B1", Some("B"), at(10.0, 8.0), 1.0, 0)
        .build()
}

fn track(design: &Design, net: &str, layer: u32, path: &[Point]) -> Track {
    Track {
        net: design.net_named(net),
        layer: LayerId(layer),
        width: mm(0.2),
        path: path.to_vec(),
    }
}

fn check(design: &Design, routes: Routes) -> Verdict {
    verify(design, &compile(design, Purpose::Verification), &routes)
}

#[test]
fn a_clean_route_passes() {
    let design = board();
    let routes = Routes {
        tracks: vec![track(&design, "A", 0, &[at(2.0, 5.0), at(18.0, 5.0)])],
        vias: vec![],
    };
    assert_eq!(check(&design, routes), Verdict::default());
}

#[test]
fn missing_copper_leaves_pads_unconnected() {
    let design = board();
    let verdict = check(&design, Routes::default());
    assert_eq!(
        verdict.violations,
        [Violation::Unconnected {
            net: design.net_named("A"),
            pads: vec![design.pad_named("A2")]
        }]
    );
}

#[test]
fn a_track_too_close_to_another_net_is_reported_once() {
    let design = board();
    let routes = Routes {
        tracks: vec![track(
            &design,
            "A",
            0,
            &[at(2.0, 5.0), at(8.0, 7.6), at(12.0, 7.6), at(18.0, 5.0)],
        )],
        vias: vec![],
    };
    let clearance: Vec<_> = check(&design, routes)
        .violations
        .into_iter()
        .filter(|v| matches!(v, Violation::Clearance { .. }))
        .collect();
    assert_eq!(
        clearance,
        [Violation::Clearance {
            layer: LayerId(0),
            a: CopperItem::Pad(design.pad_named("B1")),
            b: CopperItem::Track(0),
            required: mm(0.2)
        }]
    );
}

#[test]
fn copper_near_the_edge_is_reported() {
    let design = board();
    let routes = Routes {
        tracks: vec![track(
            &design,
            "A",
            0,
            &[at(2.0, 5.0), at(2.0, 0.2), at(18.0, 0.2), at(18.0, 5.0)],
        )],
        vias: vec![],
    };
    assert!(
        check(&design, routes)
            .violations
            .contains(&Violation::Edge {
                layer: LayerId(0),
                item: CopperItem::Track(0)
            })
    );
}

#[test]
fn copper_near_a_cutout_inside_the_board_is_reported() {
    use traxide_contracts::geometry::{Polygon, Rect};
    // A cutout from (9, 6) to (11, 9) mm, as a footprint may draw on Edge.Cuts;
    // the track passes 0.1 mm below it, closer than the 0.2 mm edge clearance.
    let mut design = board();
    let cutout = Polygon::rectangle(Rect {
        min: at(9.0, 6.0),
        max: at(11.0, 9.0),
    });
    design.outline = design.outline.clone().with_holes(vec![cutout]);
    let routes = Routes {
        tracks: vec![track(&design, "A", 0, &[at(2.0, 5.0), at(18.0, 5.0)])],
        vias: vec![],
    };
    assert!(
        check(&design, routes.clone()).is_clean(),
        "5 mm is clear of it"
    );
    let close = Routes {
        tracks: vec![track(
            &design,
            "A",
            0,
            &[
                at(2.0, 5.0),
                at(8.0, 5.0),
                at(8.0, 5.8),
                at(12.0, 5.8),
                at(12.0, 5.0),
                at(18.0, 5.0),
            ],
        )],
        vias: vec![],
    };
    assert!(check(&design, close).violations.contains(&Violation::Edge {
        layer: LayerId(0),
        item: CopperItem::Track(0)
    }));
}

#[test]
fn vias_connect_the_layers_they_span() {
    let design = board();
    let a = design.net_named("A");
    let routes = Routes {
        tracks: vec![
            track(&design, "A", 0, &[at(2.0, 5.0), at(5.0, 5.0)]),
            track(&design, "A", 1, &[at(5.0, 5.0), at(15.0, 5.0)]),
            track(&design, "A", 0, &[at(15.0, 5.0), at(18.0, 5.0)]),
        ],
        vias: vec![
            Via {
                net: a,
                kind: ViaKindId(0),
                position: at(5.0, 5.0),
            },
            Via {
                net: a,
                kind: ViaKindId(0),
                position: at(15.0, 5.0),
            },
        ],
    };
    assert_eq!(check(&design, routes), Verdict::default());
}

#[test]
fn copper_on_the_wrong_layer_does_not_connect() {
    let design = board();
    let routes = Routes {
        tracks: vec![track(&design, "A", 1, &[at(2.0, 5.0), at(18.0, 5.0)])],
        vias: vec![],
    };
    assert!(matches!(
        check(&design, routes).violations[..],
        [
            Violation::Dangling { .. },
            Violation::Dangling { .. },
            Violation::Unconnected { .. }
        ]
    ));
}

#[test]
fn pads_of_different_nets_closer_than_their_clearance_are_named_once_a_pair() {
    // A1 and B1 through both layers, 0.1 mm apart; A1 and A2 as close, but of one net.
    let design = DesignBuilder::new(20.0, 10.0)
        .tht("A1", Some("A"), at(5.0, 5.0), 1.0)
        .tht("B1", Some("B"), at(6.1, 5.0), 1.0)
        .tht("A2", Some("A"), at(3.9, 5.0), 1.0)
        .tht("C1", Some("C"), at(15.0, 5.0), 1.0)
        .build();
    let rules = compile(&design, Purpose::Verification);
    assert_eq!(
        traxide_verification::pads_too_close(&design, &rules),
        [[design.pad_named("A1"), design.pad_named("B1")]]
    );
}

#[test]
fn pads_of_one_part_are_named_only_where_they_overlap() {
    // A footprint may set its pins closer than the board's clearance: U1's are
    // 0.1 mm apart. U2's overlap, which shorts their nets.
    let design = DesignBuilder::new(20.0, 10.0)
        .smd("U1-1", Some("A"), at(5.0, 5.0), 0.5, 0)
        .smd("U1-2", Some("B"), at(5.6, 5.0), 0.5, 0)
        .smd("U2-1", Some("A"), at(10.0, 5.0), 0.5, 0)
        .smd("U2-2", Some("B"), at(10.4, 5.0), 0.5, 0)
        .build();
    let rules = compile(&design, Purpose::Verification);
    assert_eq!(
        traxide_verification::pads_too_close(&design, &rules),
        [[design.pad_named("U2-1"), design.pad_named("U2-2")]]
    );
}

/// A 3-layer board whose middle layer is a GND plane over its left half, with
/// a GND pad on the top layer over the plane and one outside it.
fn board_with_a_plane() -> DesignBuilder {
    DesignBuilder::new(20.0, 10.0)
        .layers(3)
        .plane_layer(1)
        .plane("GND", 1, at(0.0, 0.0), at(10.0, 10.0))
        .smd("G1", Some("GND"), at(2.0, 5.0), 1.0, 0)
}

fn via(design: &Design, net: &str, position: Point) -> Via {
    Via {
        net: design.net_named(net),
        kind: ViaKindId(0),
        position,
    }
}

#[test]
fn a_plane_joins_the_through_hole_pads_and_vias_of_its_net_on_it() {
    let design = board_with_a_plane()
        .tht("J1", Some("GND"), at(8.0, 2.0), 1.6)
        .build();
    let routes = Routes {
        tracks: vec![track(&design, "GND", 0, &[at(2.0, 5.0), at(3.0, 5.0)])],
        vias: vec![via(&design, "GND", at(3.0, 5.0))],
    };
    assert_eq!(check(&design, routes), Verdict::default());
}

#[test]
fn copper_outside_the_plane_is_not_joined_by_it() {
    let design = board_with_a_plane()
        .tht("J1", Some("GND"), at(15.0, 2.0), 1.6)
        .build();
    let routes = Routes {
        tracks: vec![track(&design, "GND", 0, &[at(2.0, 5.0), at(3.0, 5.0)])],
        vias: vec![via(&design, "GND", at(3.0, 5.0))],
    };
    assert_eq!(
        check(&design, routes).violations,
        [Violation::Unconnected {
            net: design.net_named("GND"),
            pads: vec![design.pad_named("J1")]
        }]
    );
}

#[test]
fn a_plane_joins_nothing_where_a_plane_of_another_net_overlaps_it() {
    // Which of two overlapping pours fills the overlap is the board program's
    // choice: their order of priority, which the design does not know.
    let design = board_with_a_plane()
        .plane("VCC", 1, at(6.0, 0.0), at(10.0, 4.0))
        .tht("J1", Some("GND"), at(8.0, 2.0), 1.6)
        .tht("V1", Some("VCC"), at(18.0, 8.0), 1.6)
        .build();
    let routes = Routes {
        tracks: vec![track(&design, "GND", 0, &[at(2.0, 5.0), at(3.0, 5.0)])],
        vias: vec![via(&design, "GND", at(3.0, 5.0))],
    };
    assert_eq!(
        check(&design, routes).violations,
        [Violation::Unconnected {
            net: design.net_named("GND"),
            pads: vec![design.pad_named("J1")]
        }]
    );
}

#[test]
fn a_pour_on_a_layer_that_takes_tracks_joins_nothing() {
    // Tracks of other nets may cut such a pour apart, wherever routing put them.
    let design = DesignBuilder::new(20.0, 10.0)
        .plane("GND", 1, at(0.0, 0.0), at(20.0, 10.0))
        .tht("J1", Some("GND"), at(2.0, 2.0), 1.6)
        .tht("J2", Some("GND"), at(18.0, 2.0), 1.6)
        .build();
    assert_eq!(
        check(&design, Routes::default()).violations,
        [Violation::Unconnected {
            net: design.net_named("GND"),
            pads: vec![design.pad_named("J2")]
        }]
    );
}

#[test]
fn a_rule_on_the_footprint_that_draws_a_cutout_sets_the_clearance_from_it() {
    use traxide_contracts::geometry::{Polygon, Rect};
    use traxide_contracts::{Condition, Constraint, CustomRule, Item, Limits};
    // As TokenBoard's rule "LED cutout": 0.05 mm from the cutouts of LED
    // footprints, the board's 0.2 mm elsewhere. The track passes 0.1 mm below.
    let with_cutout_of = |footprint: Option<&str>| {
        let mut design = board();
        let cutout = Polygon::rectangle(Rect {
            min: at(9.0, 6.0),
            max: at(11.0, 9.0),
        });
        design.outline = design.outline.clone().with_holes(vec![cutout]);
        design.cutout_footprints = vec![footprint.map(str::to_owned)];
        design.rules.custom = vec![CustomRule {
            name: "LED cutout".into(),
            condition: Condition::Any(vec![
                Condition::MemberOfFootprint {
                    item: Item::A,
                    footprints: vec!["LED1".into()],
                },
                Condition::MemberOfFootprint {
                    item: Item::B,
                    footprints: vec!["LED1".into()],
                },
            ]),
            constraint: Constraint::EdgeClearance,
            limits: Limits {
                min: Some(mm(0.05)),
                ..Limits::default()
            },
        }];
        design
    };
    let close = |design: &Design| Routes {
        tracks: vec![track(
            design,
            "A",
            0,
            &[
                at(2.0, 5.0),
                at(8.0, 5.0),
                at(8.0, 5.8),
                at(12.0, 5.8),
                at(12.0, 5.0),
                at(18.0, 5.0),
            ],
        )],
        vias: vec![],
    };
    let led = with_cutout_of(Some("LED1"));
    assert!(check(&led, close(&led)).is_clean());
    for other in [with_cutout_of(Some("U1")), with_cutout_of(None)] {
        assert!(!check(&other, close(&other)).is_clean());
    }
}

#[test]
fn a_plane_joins_nothing_that_other_nets_copper_walls_off_from_its_fill() {
    // The plane is filled around other nets' copper, keeping their clearance:
    // twelve VCC vias 1.6 mm around J1 leave no gap the fill can pass, so J1
    // lies in an island of its own, as KiCad fills it.
    let design = board_with_a_plane()
        .tht("J1", Some("GND"), at(6.0, 5.0), 1.6)
        .tht("V1", Some("VCC"), at(18.0, 8.0), 1.6)
        .build();
    let ring: Vec<Via> = (0..12)
        .map(|i| {
            let angle = f64::from(i) * std::f64::consts::TAU / 12.0;
            via(
                &design,
                "VCC",
                at(6.0 + 1.6 * angle.cos(), 5.0 + 1.6 * angle.sin()),
            )
        })
        .collect();
    let routes = Routes {
        tracks: vec![track(&design, "GND", 0, &[at(2.0, 5.0), at(3.0, 5.0)])],
        vias: [vec![via(&design, "GND", at(3.0, 5.0))], ring].concat(),
    };
    assert_eq!(
        check(&design, routes).violations,
        [Violation::Unconnected {
            net: design.net_named("GND"),
            pads: vec![design.pad_named("J1")]
        }]
    );
}

#[test]
fn a_plane_keeps_its_clearance_from_the_planes_of_other_nets() {
    // Two VCC pours leave a corridor of 0.3 mm in the GND plane, narrower than
    // its clearance from them on both sides: the fill does not reach J1's via.
    let design = board_with_a_plane()
        .plane("VCC", 1, at(6.0, 0.0), at(10.0, 1.85))
        .plane("VCC", 1, at(6.0, 2.15), at(10.0, 10.0))
        .smd("J1", Some("GND"), at(8.0, 2.0), 0.4, 0)
        .tht("V1", Some("VCC"), at(18.0, 8.0), 1.6)
        .build();
    let routes = Routes {
        tracks: vec![track(&design, "GND", 0, &[at(2.0, 5.0), at(3.0, 5.0)])],
        vias: vec![
            via(&design, "GND", at(3.0, 5.0)),
            via(&design, "GND", at(8.0, 2.0)),
        ],
    };
    assert_eq!(
        check(&design, routes).violations,
        [Violation::Unconnected {
            net: design.net_named("GND"),
            pads: vec![design.pad_named("J1")]
        }]
    );
}

#[test]
fn a_plane_joins_nothing_in_a_window_cut_out_of_it() {
    use traxide_contracts::geometry::{Polygon, Rect};
    let mut design = board_with_a_plane()
        .tht("J1", Some("GND"), at(7.0, 5.0), 1.6)
        .build();
    design.planes[0].holes = vec![Polygon::rectangle(Rect {
        min: at(5.0, 3.0),
        max: at(9.0, 7.0),
    })];
    let routes = Routes {
        tracks: vec![track(&design, "GND", 0, &[at(2.0, 5.0), at(3.0, 5.0)])],
        vias: vec![via(&design, "GND", at(3.0, 5.0))],
    };
    assert_eq!(
        check(&design, routes).violations,
        [Violation::Unconnected {
            net: design.net_named("GND"),
            pads: vec![design.pad_named("J1")]
        }]
    );
}

#[test]
fn copper_keeps_a_keepouts_clearance_from_it() {
    use traxide_contracts::KeepoutScope;
    use traxide_contracts::geometry::{Polygon, Rect, Shape};
    // As TokenBoard's LED cut-outs come in a DSN: keepouts that ask 0.2 mm.
    let mut design = board();
    design.keepouts = vec![traxide_contracts::Keepout {
        layer: None,
        shape: Shape::Polygon(Polygon::rectangle(Rect {
            min: at(9.0, 6.0),
            max: at(11.0, 9.0),
        })),
        scope: KeepoutScope::Everything,
        clearance: mm(0.2),
    }];
    // The track's edge 0.1 mm below the keepout.
    let close = Routes {
        tracks: vec![track(
            &design,
            "A",
            0,
            &[
                at(2.0, 5.0),
                at(8.0, 5.0),
                at(8.0, 5.8),
                at(12.0, 5.8),
                at(12.0, 5.0),
                at(18.0, 5.0),
            ],
        )],
        vias: vec![],
    };
    assert!(
        check(&design, close)
            .violations
            .contains(&Violation::Keepout {
                layer: LayerId(0),
                item: CopperItem::Track(0)
            })
    );
}

#[test]
fn vias_closer_than_the_hole_to_hole_distance_are_reported_whatever_their_nets() {
    let mut design = board();
    design.rules.hole_to_hole = mm(0.25);
    // Holes of 0.3 mm: 0.5 mm apart leaves 0.2 mm between them, 0.6 mm 0.3 mm.
    let routes = Routes {
        tracks: vec![track(&design, "A", 0, &[at(2.0, 5.0), at(18.0, 5.0)])],
        vias: vec![
            via(&design, "A", at(5.0, 5.0)),
            via(&design, "A", at(5.5, 5.0)),
            via(&design, "A", at(12.0, 5.0)),
            via(&design, "A", at(12.6, 5.0)),
        ],
    };
    assert_eq!(
        check(&design, routes).violations,
        [Violation::HoleToHole {
            a: CopperItem::Via(0),
            b: CopperItem::Via(1),
            required: mm(0.25),
        }]
    );
}

#[test]
fn copper_keeps_the_edge_clearance_from_where_the_true_edge_may_lie() {
    // A track exactly the edge clearance from the outline as read, which may
    // lie up to 2 µm inside the board's true edge, as an arc read as chords.
    let mut design = board();
    let tolerance = 2_000;
    let edge = mm(0.2) + mm(0.2) / 2;
    let routes = |design: &Design| Routes {
        tracks: vec![track(
            design,
            "A",
            0,
            &[
                at(2.0, 5.0),
                Point::new(mm(2.0), edge),
                Point::new(mm(18.0), edge),
                at(18.0, 5.0),
            ],
        )],
        vias: vec![],
    };
    let at_the_edge = |verdict: Verdict| {
        verdict.violations.contains(&Violation::Edge {
            layer: LayerId(0),
            item: CopperItem::Track(0),
        })
    };
    assert!(!at_the_edge(check(&design, routes(&design))));
    design.outline_tolerance = tolerance;
    assert!(at_the_edge(check(&design, routes(&design))));
}

#[test]
fn a_track_end_that_touches_no_copper_of_its_net_dangles() {
    // A stub from the middle of A's route that ends in nothing.
    let design = board();
    let routes = Routes {
        tracks: vec![
            track(&design, "A", 0, &[at(2.0, 5.0), at(18.0, 5.0)]),
            track(&design, "A", 0, &[at(10.0, 5.0), at(10.0, 3.0)]),
        ],
        vias: vec![],
    };
    assert_eq!(
        check(&design, routes).violations,
        [Violation::Dangling {
            layer: LayerId(0),
            item: CopperItem::Track(1),
            at: at(10.0, 3.0)
        }]
    );
}

#[test]
fn a_track_end_on_copper_of_another_net_or_layer_dangles() {
    // A's stub ends on B1, but B1 is another net's; another stub runs on layer
    // 1 from under A1, which is on layer 0 only.
    let design = board();
    let routes = Routes {
        tracks: vec![
            track(&design, "A", 0, &[at(2.0, 5.0), at(18.0, 5.0)]),
            track(&design, "A", 0, &[at(10.0, 5.0), at(10.0, 8.0)]),
            track(&design, "A", 1, &[at(2.0, 5.0), at(4.0, 5.0)]),
        ],
        vias: vec![],
    };
    let dangling: Vec<_> = check(&design, routes)
        .violations
        .into_iter()
        .filter(|v| matches!(v, Violation::Dangling { .. }))
        .collect();
    assert_eq!(
        dangling,
        [
            Violation::Dangling {
                layer: LayerId(0),
                item: CopperItem::Track(1),
                at: at(10.0, 8.0)
            },
            Violation::Dangling {
                layer: LayerId(1),
                item: CopperItem::Track(2),
                at: at(2.0, 5.0)
            },
            Violation::Dangling {
                layer: LayerId(1),
                item: CopperItem::Track(2),
                at: at(4.0, 5.0)
            },
        ]
    );
}

#[test]
fn a_track_end_in_a_pour_of_its_net_on_its_layer_does_not_dangle() {
    // As KiCad's fill joins it, wherever routing left room for the pour.
    let design = DesignBuilder::new(20.0, 10.0)
        .plane("GND", 0, at(10.0, 0.0), at(20.0, 10.0))
        .smd("G1", Some("GND"), at(2.0, 5.0), 1.0, 0)
        .build();
    let routes = Routes {
        tracks: vec![track(&design, "GND", 0, &[at(2.0, 5.0), at(15.0, 5.0)])],
        vias: vec![],
    };
    assert_eq!(check(&design, routes), Verdict::default());
}

#[test]
fn a_track_end_just_off_a_pads_edge_that_its_width_overlaps_does_not_dangle() {
    // A session's resolution moves an end on the pad's edge off it by a step.
    let design = board();
    let routes = Routes {
        tracks: vec![track(&design, "A", 0, &[at(2.55, 5.0), at(17.45, 5.0)])],
        vias: vec![],
    };
    assert_eq!(check(&design, routes), Verdict::default());
}

#[test]
fn a_track_narrower_than_the_boards_minimum_breaks_the_width_rule() {
    let mut design = board();
    design.rules.min_track_width = mm(0.25);
    let routes = Routes {
        tracks: vec![track(&design, "A", 0, &[at(2.0, 5.0), at(18.0, 5.0)])],
        vias: vec![],
    };
    assert_eq!(
        check(&design, routes).violations,
        [Violation::TrackWidth {
            item: CopperItem::Track(0),
            width: mm(0.2),
            min: Some(mm(0.25)),
            max: None
        }]
    );
}

#[test]
fn a_custom_rule_sets_the_width_limits_of_the_tracks_it_names() {
    // A's tracks between 0.3 mm and 0.5 mm wide, as a KiCad board's rules may ask.
    let mut design = board();
    let a = design.net_named("A");
    design.rules.custom.push(traxide_contracts::CustomRule {
        name: "power".into(),
        condition: traxide_contracts::Condition::NetIn {
            item: traxide_contracts::Item::A,
            nets: vec![Some(a)],
        },
        constraint: traxide_contracts::Constraint::TrackWidth,
        limits: traxide_contracts::Limits {
            min: Some(mm(0.3)),
            opt: None,
            max: Some(mm(0.5)),
        },
    });
    let mut narrow = track(&design, "A", 0, &[at(2.0, 5.0), at(10.0, 5.0)]);
    narrow.width = mm(0.2);
    let mut wide = track(&design, "A", 0, &[at(10.0, 5.0), at(18.0, 5.0)]);
    wide.width = mm(0.6);
    let routes = Routes {
        tracks: vec![narrow, wide],
        vias: vec![],
    };
    let widths: Vec<_> = check(&design, routes)
        .violations
        .into_iter()
        .filter(|v| matches!(v, Violation::TrackWidth { .. }))
        .collect();
    assert_eq!(
        widths,
        [
            Violation::TrackWidth {
                item: CopperItem::Track(0),
                width: mm(0.2),
                min: Some(mm(0.3)),
                max: Some(mm(0.5))
            },
            Violation::TrackWidth {
                item: CopperItem::Track(1),
                width: mm(0.6),
                min: Some(mm(0.3)),
                max: Some(mm(0.5))
            },
        ]
    );
}

#[test]
fn a_class_width_alone_limits_nothing() {
    // Specctra's class widths are the widths to route at, not limits.
    let design = board();
    let mut thin = track(&design, "A", 0, &[at(2.0, 5.0), at(18.0, 5.0)]);
    thin.width = mm(0.1);
    let routes = Routes {
        tracks: vec![thin],
        vias: vec![],
    };
    assert_eq!(check(&design, routes), Verdict::default());
}
