use traxide_contracts::geometry::Point;
use traxide_contracts::testing::{DesignBuilder, at, mm};
use traxide_contracts::{Design, LayerId, Routes, Track};
use traxide_layout_access::Layout;
use traxide_rules::Purpose;

/// The wiring of net A along `points`, one track of two points for each
/// segment, as a board's program stores it.
fn segments(design: &Design, points: &[Point]) -> Vec<Track> {
    points
        .windows(2)
        .map(|w| Track {
            net: design.net_named("A"),
            layer: LayerId::from_index(0),
            width: mm(0.25),
            path: vec![w[0], w[1]],
        })
        .collect()
}

fn optimised(design: &Design, routes: &Routes) -> Routes {
    let rules = traxide_rules::compile(
        design,
        Purpose::Routing {
            grid_pitch: 100_000,
        },
    );
    let mut layout = Layout::build(design, &rules);
    traxide_optimization::optimise(design, routes, &mut layout)
}

fn length(routes: &Routes) -> i64 {
    (routes.tracks.iter())
        .flat_map(|t| t.path.windows(2))
        .map(|w| w[0].distance(w[1]))
        .sum()
}

/// No rule broken and every pad joined, judged by the Verification Engine.
fn assert_legal(design: &Design, routes: &Routes) {
    let rules = traxide_rules::compile(design, Purpose::Verification);
    let verdict = traxide_verification::verify(design, &rules, routes);
    assert!(verdict.violations.is_empty(), "{:?}", verdict.violations);
}

/// A detour from A1 to A2 through (5, 15) and (15, 15), in five segments.
const DETOUR: [(f64, f64); 6] = [
    (2.0, 10.0),
    (5.0, 10.0),
    (5.0, 15.0),
    (15.0, 15.0),
    (15.0, 10.0),
    (18.0, 10.0),
];

fn two_pads() -> DesignBuilder {
    DesignBuilder::new(20.0, 20.0)
        .smd("A1", Some("A"), at(2.0, 10.0), 1.0, 0)
        .smd("A2", Some("A"), at(18.0, 10.0), 1.0, 0)
}

fn points(of: &[(f64, f64)]) -> Vec<Point> {
    of.iter().map(|&(x, y)| at(x, y)).collect()
}

#[test]
fn segments_meeting_end_to_end_are_pulled_tight_as_one_track() {
    let design = two_pads().build();
    let routes = Routes {
        tracks: segments(&design, &points(&DETOUR)),
        vias: Vec::new(),
    };
    let tight = optimised(&design, &routes);
    assert!(length(&tight) <= i64::from(mm(16.0)) + 1_000, "{tight:?}");
    assert_legal(&design, &tight);
}

#[test]
fn segments_meeting_on_a_pad_of_their_net_stay_on_it() {
    let design = two_pads()
        .smd("A3", Some("A"), at(10.0, 15.0), 1.0, 0)
        .build();
    let mut through_a3 = points(&DETOUR);
    through_a3.insert(3, at(10.0, 15.0));
    let routes = Routes {
        tracks: segments(&design, &through_a3),
        vias: Vec::new(),
    };
    let tight = optimised(&design, &routes);
    assert!(length(&tight) < length(&routes), "{tight:?}");
    assert_legal(&design, &tight);
}

#[test]
fn a_branch_stays_joined_where_it_meets_the_segments() {
    let design = two_pads()
        .smd("A3", Some("A"), at(10.0, 18.0), 1.0, 0)
        .build();
    let mut wiring = points(&DETOUR);
    wiring.insert(3, at(10.0, 15.0));
    let mut tracks = segments(&design, &wiring);
    tracks.extend(segments(&design, &[at(10.0, 15.0), at(10.0, 18.0)]));
    let routes = Routes {
        tracks,
        vias: Vec::new(),
    };
    let tight = optimised(&design, &routes);
    assert!(length(&tight) < length(&routes), "{tight:?}");
    assert_legal(&design, &tight);
}
