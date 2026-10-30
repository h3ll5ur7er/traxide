use traxide_contracts::geometry::Point;
use traxide_contracts::testing::{DesignBuilder, at, mm};
use traxide_contracts::{Design, LayerId, Routes, Track, Via, ViaKindId};
use traxide_layout_access::Layout;
use traxide_rules::Purpose;

fn rules(design: &Design) -> traxide_contracts::ConstraintSet {
    traxide_rules::compile(
        design,
        Purpose::Routing {
            grid_pitch: 100_000,
        },
    )
}

fn track(design: &Design, net: &str, layer: usize, path: &[Point]) -> Track {
    Track {
        net: design.net_named(net),
        layer: LayerId::from_index(layer),
        width: mm(0.25),
        path: path.to_vec(),
    }
}

fn pulled_tight(design: &Design, routes: &Routes) -> Routes {
    let rules = rules(design);
    let mut layout = Layout::build(design, &rules);
    traxide_optimization::pull_tight(routes, &mut layout)
}

fn length(path: &[Point]) -> f64 {
    path.windows(2)
        .map(|w| {
            let (dx, dy) = (f64::from(w[1].x - w[0].x), f64::from(w[1].y - w[0].y));
            dx.hypot(dy)
        })
        .sum()
}

fn assert_legal(design: &Design, routes: &Routes) {
    let rules = traxide_rules::compile(design, Purpose::Verification);
    let mut everything = design.existing.clone();
    everything.extend(routes.clone());
    let verdict = traxide_verification::verify(design, &rules, &everything);
    assert!(verdict.violations.is_empty(), "{:?}", verdict.violations);
}

/// Each corner turns by 90° at most: a sharper one traps etchant.
fn assert_no_acute_corners(path: &[Point]) {
    for w in path.windows(3) {
        let (u, v) = (w[1] - w[0], w[2] - w[1]);
        let dot = i64::from(u.x) * i64::from(v.x) + i64::from(u.y) * i64::from(v.y);
        assert!(dot >= 0, "an acute corner at {:?} in {path:?}", w[1]);
    }
}

fn two_pads() -> DesignBuilder {
    DesignBuilder::new(20.0, 20.0)
        .smd("A1", Some("A"), at(2.0, 10.0), 1.0, 0)
        .smd("A2", Some("A"), at(18.0, 10.0), 1.0, 0)
}

#[test]
fn a_detour_through_free_room_is_pulled_straight() {
    let design = two_pads().build();
    let detour = track(
        &design,
        "A",
        0,
        &[at(2.0, 10.0), at(5.0, 13.0), at(15.0, 13.0), at(18.0, 10.0)],
    );
    let routes = Routes {
        tracks: vec![detour],
        vias: Vec::new(),
    };
    let tight = pulled_tight(&design, &routes);
    assert_eq!(tight.tracks[0].path, [at(2.0, 10.0), at(18.0, 10.0)]);
    assert_legal(&design, &tight);
}

#[test]
fn a_track_pulled_tight_keeps_clear_of_other_nets_copper() {
    let design = two_pads()
        .smd("B1", Some("B"), at(10.0, 10.0), 2.0, 0)
        .build();
    let around = [at(2.0, 10.0), at(2.0, 14.0), at(18.0, 14.0), at(18.0, 10.0)];
    let routes = Routes {
        tracks: vec![track(&design, "A", 0, &around)],
        vias: Vec::new(),
    };
    let tight = pulled_tight(&design, &routes);
    let path = &tight.tracks[0].path;
    assert!(length(path) < length(&around), "{path:?}");
    assert_eq!(
        (path.first(), path.last()),
        (around.first(), around.last()),
        "its ends stay where they were"
    );
    assert_no_acute_corners(path);
    assert_legal(&design, &tight);
}

#[test]
fn a_track_moved_makes_room_for_the_next_one() {
    // The second track is checked against the first as it is after pulling
    // tight, not as it was.
    let design = two_pads()
        .smd("C1", Some("C"), at(2.0, 12.0), 1.0, 0)
        .smd("C2", Some("C"), at(18.0, 12.0), 1.0, 0)
        .build();
    let routes = Routes {
        tracks: vec![
            track(
                &design,
                "A",
                0,
                &[at(2.0, 10.0), at(4.0, 8.0), at(16.0, 8.0), at(18.0, 10.0)],
            ),
            track(
                &design,
                "C",
                0,
                &[at(2.0, 12.0), at(4.0, 14.0), at(16.0, 14.0), at(18.0, 12.0)],
            ),
        ],
        vias: Vec::new(),
    };
    let tight = pulled_tight(&design, &routes);
    assert_eq!(tight.tracks[0].path, [at(2.0, 10.0), at(18.0, 10.0)]);
    assert_eq!(tight.tracks[1].path, [at(2.0, 12.0), at(18.0, 12.0)]);
    assert_legal(&design, &tight);
}

#[test]
fn vias_and_the_tracks_ending_at_them_stay_joined() {
    let design = DesignBuilder::new(20.0, 20.0)
        .layers(2)
        .smd("A1", Some("A"), at(2.0, 10.0), 1.0, 0)
        .smd("A2", Some("A"), at(18.0, 10.0), 1.0, 1)
        .build();
    let via = at(10.0, 10.0);
    let routes = Routes {
        tracks: vec![
            track(
                &design,
                "A",
                0,
                &[at(2.0, 10.0), at(4.0, 12.0), at(8.0, 12.0), via],
            ),
            track(
                &design,
                "A",
                1,
                &[via, at(12.0, 8.0), at(16.0, 8.0), at(18.0, 10.0)],
            ),
        ],
        vias: vec![Via {
            net: design.net_named("A"),
            kind: ViaKindId::from_index(0),
            position: via,
        }],
    };
    let tight = pulled_tight(&design, &routes);
    assert_eq!(tight.vias, routes.vias);
    assert_eq!(tight.tracks[0].path, [at(2.0, 10.0), via]);
    assert_eq!(tight.tracks[1].path, [via, at(18.0, 10.0)]);
    assert_legal(&design, &tight);
}

#[test]
fn a_loop_in_a_track_is_cut_out() {
    let design = two_pads().build();
    let looped = [
        at(2.0, 10.0),
        at(6.0, 10.0),
        at(6.0, 12.0),
        at(8.0, 12.0),
        at(8.0, 10.0),
        at(6.0, 10.0),
        at(18.0, 10.0),
    ];
    let routes = Routes {
        tracks: vec![track(&design, "A", 0, &looped)],
        vias: Vec::new(),
    };
    let tight = pulled_tight(&design, &routes);
    assert_eq!(tight.tracks[0].path, [at(2.0, 10.0), at(18.0, 10.0)]);
}

#[test]
fn a_shortcut_that_runs_on_in_the_tracks_direction_leaves_no_corner() {
    let design = two_pads().build();
    let jog = [
        at(2.0, 10.0),
        at(4.0, 12.0),
        at(6.0, 12.0),
        at(8.0, 10.0),
        at(18.0, 10.0),
    ];
    let routes = Routes {
        tracks: vec![track(&design, "A", 0, &jog)],
        vias: Vec::new(),
    };
    let tight = pulled_tight(&design, &routes);
    assert_eq!(tight.tracks[0].path, [at(2.0, 10.0), at(18.0, 10.0)]);
}

#[test]
fn a_track_of_its_net_ending_on_another_stays_joined_to_it() {
    // B ends on A's track where A detours: pulled straight, A would leave B's
    // end in the air, so A keeps that point.
    let design = two_pads()
        .smd("A3", Some("A"), at(10.0, 17.0), 1.0, 0)
        .build();
    let joint = at(10.0, 13.0);
    let routes = Routes {
        tracks: vec![
            track(
                &design,
                "A",
                0,
                &[at(2.0, 10.0), at(5.0, 13.0), at(15.0, 13.0), at(18.0, 10.0)],
            ),
            track(&design, "A", 0, &[at(10.0, 17.0), joint]),
        ],
        vias: Vec::new(),
    };
    let tight = pulled_tight(&design, &routes);
    let on_a = tight.tracks[0].path.windows(2).any(|w| {
        let (ab, aj) = (w[1] - w[0], joint - w[0]);
        i64::from(ab.x) * i64::from(aj.y) == i64::from(ab.y) * i64::from(aj.x)
            && (w[0].x.min(w[1].x)..=w[0].x.max(w[1].x)).contains(&joint.x)
            && (w[0].y.min(w[1].y)..=w[0].y.max(w[1].y)).contains(&joint.y)
    });
    assert!(on_a, "{:?}", tight.tracks[0].path);
    assert_eq!(tight.tracks[1].path.last(), Some(&joint));
    assert_legal(&design, &tight);
}
