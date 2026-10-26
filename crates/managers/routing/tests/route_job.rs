use std::cell::RefCell;
use std::path::PathBuf;
use traxide_contracts::{Event, Stage};
use traxide_events::{CancelToken, Silent};
use traxide_routing::{RouteJob, RouteReport, Unroutable, route};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(format!(
        "{}/../../../fixtures/boards/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
}

fn job(input: &str, output: &std::path::Path) -> RouteJob {
    RouteJob {
        input: fixture(input),
        output: Some(output.into()),
        profile: "default".into(),
        profile_file: None,
        threads: None,
        unrouted_net_classes: Vec::new(),
        drc_report: None,
        time_limit: None,
        reroute_existing: false,
    }
}

#[test]
fn routes_every_connection_and_stores_a_clean_session() {
    let folder = tempfile::tempdir().expect("temp dir");
    let output = folder.path().join("board.ses");
    let report =
        route(&job("two-nets.dsn", &output), &Silent, &CancelToken::new()).expect("routable");
    assert_eq!((report.connections, report.unrouted.len()), (2, 0));
    assert!(report.verdict.is_clean(), "{:?}", report.verdict);
    assert!(
        std::fs::read_to_string(output)
            .expect("stored")
            .contains("(network_out")
    );
}

#[test]
fn tells_its_client_what_it_is_doing() {
    let folder = tempfile::tempdir().expect("temp dir");
    let events = RefCell::new(Vec::new());
    let collect = |e: Event| events.borrow_mut().push(e);
    route(
        &job("two-nets.dsn", &folder.path().join("board.ses")),
        &collect,
        &CancelToken::new(),
    )
    .expect("routable");
    let events = events.into_inner();
    assert_eq!(events.first(), Some(&Event::StageStarted(Stage::Loading)));
    assert!(events.contains(&Event::ConnectionRouted { done: 2, total: 2 }));
    assert_eq!(events.last(), Some(&Event::StageStarted(Stage::Storing)));
}

#[test]
fn a_cancelled_job_stores_what_it_has_and_lists_the_rest_as_unrouted() {
    let folder = tempfile::tempdir().expect("temp dir");
    let cancel = CancelToken::new();
    cancel.cancel();
    let report = route(
        &job("two-nets.dsn", &folder.path().join("board.ses")),
        &Silent,
        &cancel,
    )
    .expect("stores");
    assert!(report.cancelled);
    assert_eq!(report.unrouted.len(), 2);
}

#[test]
fn a_missing_board_is_an_error_that_names_it() {
    let folder = tempfile::tempdir().expect("temp dir");
    let error = route(
        &job("missing.dsn", &folder.path().join("board.ses")),
        &Silent,
        &CancelToken::new(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("missing.dsn"), "{error}");
}

#[test]
fn nets_of_the_net_classes_left_unrouted_get_no_copper() {
    let folder = tempfile::tempdir().expect("temp dir");
    let output = folder.path().join("board.ses");
    let job = RouteJob {
        unrouted_net_classes: vec!["Power".into()],
        ..job("two-classes.dsn", &output)
    };
    let report = route(&job, &Silent, &CancelToken::new()).expect("routable");
    assert_eq!((report.connections, report.unrouted.len()), (1, 0));
    assert_eq!(report.tracks, 1, "only Across, straight across");
}

#[test]
fn an_unknown_net_class_to_leave_unrouted_is_an_error_naming_the_known_ones() {
    let folder = tempfile::tempdir().expect("temp dir");
    let job = RouteJob {
        unrouted_net_classes: vec!["Pwr".into()],
        ..job("two-classes.dsn", &folder.path().join("board.ses"))
    };
    let error = route(&job, &Silent, &CancelToken::new()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "no net class named Pwr; there are default, kicad_default, Power"
    );
}

#[test]
fn rules_the_board_broke_before_routing_are_not_counted_as_the_routers() {
    // An existing track of Across crosses a pad of Down.
    let folder = tempfile::tempdir().expect("temp dir");
    let output = folder.path().join("board.ses");
    let report = route(
        &job("existing-violation.dsn", &output),
        &Silent,
        &CancelToken::new(),
    )
    .expect("routable");
    assert_eq!(report.violations_before_routing, 1);
    assert!(
        report
            .verdict
            .violations
            .iter()
            .all(|v| matches!(v, traxide_contracts::Violation::Unconnected { .. })),
        "{:?}",
        report.verdict
    );
}

#[test]
fn the_drc_report_judges_the_stored_copper_with_the_boards_own() {
    let folder = tempfile::tempdir().expect("temp dir");
    let output = folder.path().join("board.ses");
    let drc = folder.path().join("board.drc.json");
    let job = RouteJob {
        drc_report: Some(drc.clone()),
        ..job("existing-violation.dsn", &output)
    };
    let report = route(&job, &Silent, &CancelToken::new()).expect("routable");
    let unconnected_pads: usize = report
        .verdict
        .violations
        .iter()
        .map(|v| match v {
            traxide_contracts::Violation::Unconnected { pads, .. } => pads.len(),
            _ => 0,
        })
        .sum();
    let drc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(drc).expect("written")).expect("JSON");
    assert_eq!(drc["source"], "board.ses");
    let types: Vec<&serde_json::Value> = drc["violations"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|v| &v["type"])
        .collect();
    assert_eq!(types, ["clearance"], "the board's own track over a pad");
    assert_eq!(
        drc["unconnected_items"].as_array().map(Vec::len),
        Some(unconnected_pads)
    );
}

#[test]
fn says_why_a_connection_stayed_unrouted() {
    // R2 lies beyond the board's right edge, R5 beyond R2.
    let folder = tempfile::tempdir().expect("temp dir");
    let report = route(
        &job("pad-off-board.dsn", &folder.path().join("board.ses")),
        &Silent,
        &CancelToken::new(),
    )
    .expect("routable");
    let reasons: Vec<(&str, &[Unroutable])> = report
        .unrouted
        .iter()
        .map(|u| (u.net.as_str(), u.why.as_slice()))
        .collect();
    assert_eq!(
        reasons,
        [
            (
                "Away",
                &[
                    Unroutable::OffTheBoard("R2-2".into()),
                    Unroutable::OffTheBoard("R5-1".into())
                ][..]
            ),
            ("Across", &[Unroutable::OffTheBoard("R2-1".into())][..]),
        ]
    );
}

#[test]
fn the_session_keeps_the_copper_the_board_came_with() {
    // KiCad's session import replaces every track of the board with the
    // session's, so a session without the board's own wiring would delete it.
    let folder = tempfile::tempdir().expect("temp dir");
    let output = folder.path().join("board.ses");
    route(
        &job("existing-violation.dsn", &output),
        &Silent,
        &CancelToken::new(),
    )
    .expect("routable");
    let design =
        traxide_board_access::load_design(&fixture("existing-violation.dsn")).expect("valid");
    let stored = traxide_board_access::load_routes(&output, &design).expect("valid session");
    for existing in &design.existing.tracks {
        assert!(stored.tracks.contains(existing), "{existing:?} kept");
    }
}

#[test]
fn routes_a_kicad_board_into_a_copy_beside_it_with_its_project() {
    let folder = tempfile::tempdir().expect("temp dir");
    for file in ["crossing.kicad_pcb", "crossing.kicad_pro"] {
        std::fs::copy(fixture(file), folder.path().join(file)).expect("copied");
    }
    let job = RouteJob {
        input: folder.path().join("crossing.kicad_pcb"),
        output: None,
        ..job("crossing.kicad_pcb", folder.path())
    };
    let report = route(&job, &Silent, &CancelToken::new()).expect("routable");
    assert!(
        report.unrouted.is_empty() && report.verdict.is_clean(),
        "{report:?}"
    );
    assert_eq!(
        report.output,
        folder.path().join("crossing.routed.kicad_pcb")
    );
    let routed = std::fs::read_to_string(&report.output).expect("stored");
    assert!(routed.matches("(segment").count() >= report.tracks && report.tracks > 0);
    assert!(folder.path().join("crossing.routed.kicad_pro").is_file());
    assert!(!report.zones_to_refill, "the board has no zones");
}

#[test]
fn a_kicad_board_file_with_copper_zones_keeps_their_fills_for_the_designer_to_refill() {
    let folder = tempfile::tempdir().expect("temp dir");
    let board = std::fs::read_to_string(fixture("crossing.kicad_pcb")).expect("fixture");
    let zone = r#"	(zone (net "DOWN") (layer "B.Cu") (uuid "0d3f6b1e-9a52-4c1d-8e0b-6c7f2a9d4e10")
		(connect_pads (clearance 0.5)) (min_thickness 0.25)
		(fill (thermal_gap 0.5) (thermal_bridge_width 0.5))
		(polygon (pts (xy 0 0) (xy 3 0) (xy 3 3) (xy 0 3))))
"#;
    let end = board
        .trim_end()
        .rfind(')')
        .expect("the board's closing parenthesis");
    std::fs::write(
        folder.path().join("crossing.kicad_pcb"),
        format!("{}{zone})\n", &board[..end]),
    )
    .expect("written");
    std::fs::copy(
        fixture("crossing.kicad_pro"),
        folder.path().join("crossing.kicad_pro"),
    )
    .expect("copied");
    let job = RouteJob {
        input: folder.path().join("crossing.kicad_pcb"),
        output: None,
        ..job("crossing.kicad_pcb", folder.path())
    };
    let report = route(&job, &Silent, &CancelToken::new()).expect("routable");
    assert!(report.zones_to_refill, "{report:?}");
    assert!(!report.zones_refilled);
}

#[test]
fn the_report_names_the_layers_kept_free_of_tracks() {
    let folder = tempfile::tempdir().expect("temp dir");
    let board = folder.path().join("planes.dsn");
    let text = std::fs::read_to_string(fixture("two-nets.dsn"))
        .expect("fixture")
        .replace("(layer B.Cu (type signal)", "(layer B.Cu (type power)")
        .replace(
            "    (via ",
            "    (plane GND (polygon B.Cu 0  30000 -20000  0 -20000  0 0  30000 0))\n    (via ",
        );
    std::fs::write(&board, text).expect("written");
    let job = RouteJob {
        input: board,
        output: Some(folder.path().join("planes.ses")),
        ..job("two-nets.dsn", folder.path())
    };
    let report = route(&job, &Silent, &CancelToken::new()).expect("routable");
    assert_eq!(report.planes, ["B.Cu"]);
}

#[test]
fn the_report_passes_on_what_the_reader_found_wrong_with_the_board() {
    let folder = tempfile::tempdir().expect("temp dir");
    let board = folder.path().join("twice.dsn");
    let text = std::fs::read_to_string(fixture("two-nets.dsn"))
        .expect("fixture")
        .replace(
            "(net Down (pins R3-1 R4-2))",
            "(net Down (pins R3-1 R4-2 R1-2))",
        );
    std::fs::write(&board, text).expect("written");
    let job = RouteJob {
        input: board,
        output: Some(folder.path().join("twice.ses")),
        ..job("two-nets.dsn", folder.path())
    };
    let report = route(&job, &Silent, &CancelToken::new()).expect("routable");
    assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
    assert!(report.warnings[0].contains("R1-2"));
}

#[test]
fn warns_of_pads_of_different_nets_closer_than_their_clearance() {
    let folder = tempfile::tempdir().expect("temp dir");
    let report = route(
        &job("pads-too-close.dsn", &folder.path().join("board.ses")),
        &Silent,
        &CancelToken::new(),
    )
    .expect("routable");
    assert_eq!(
        report.warnings,
        ["the pads R1-2 (net Across) and R3-1 (net Short) are closer than their clearance"]
    );
}

#[test]
fn says_when_the_board_itself_leaves_no_way_between_two_pads() {
    // R1 lies inside a closed ring of keepout on both layers, R2 outside it.
    let folder = tempfile::tempdir().expect("temp dir");
    let report = route(
        &job("enclosed-pad.dsn", &folder.path().join("board.ses")),
        &Silent,
        &CancelToken::new(),
    )
    .expect("routable");
    let reasons: Vec<&[Unroutable]> = report.unrouted.iter().map(|u| u.why.as_slice()).collect();
    assert_eq!(reasons, [&[Unroutable::CutOff][..]]);
}

#[test]
fn the_pins_of_a_fine_pitch_chip_escape_by_stubs_that_the_session_keeps() {
    let folder = tempfile::tempdir().expect("temp dir");
    let report = route(
        &job("fine-pitch.dsn", &folder.path().join("board.ses")),
        &Silent,
        &CancelToken::new(),
    )
    .expect("routable");
    assert_eq!((report.connections, report.unrouted.len()), (16, 0));
    assert!(report.verdict.is_clean(), "{:?}", report.verdict);
    assert!(report.tracks >= 2 * 16, "a stub and a route per pin");
}

#[test]
fn a_job_past_its_time_limit_still_stores_what_routes_without_rip_up() {
    let folder = tempfile::tempdir().expect("temp dir");
    let output = folder.path().join("board.ses");
    let job = RouteJob {
        time_limit: Some(std::time::Duration::ZERO),
        ..job("two-nets.dsn", &output)
    };
    let report = route(&job, &Silent, &CancelToken::new()).expect("routable");
    assert_eq!((report.connections, report.unrouted.len()), (2, 0));
    assert!(output.exists());
}

#[test]
fn pads_of_a_net_with_a_plane_join_it_by_vias_beside_them() {
    // GND is a plane on In1.Cu; its four pads, on both outer layers, lie far
    // apart, at the corners of the board.
    let folder = tempfile::tempdir().expect("temp dir");
    let output = folder.path().join("board.ses");
    let report =
        route(&job("gnd-plane.dsn", &output), &Silent, &CancelToken::new()).expect("routable");
    assert_eq!((report.connections, report.unrouted.len()), (2, 0));
    assert!(report.verdict.is_clean(), "{:?}", report.verdict);
    assert_eq!(report.vias, 4, "one beside each GND pad");
}

/// two-nets.dsn with a ring of the net Across's wiring round R3 on both
/// layers, typed `kind`: Down cannot leave R3 while it stays.
fn ringed_board(folder: &std::path::Path, kind: &str) -> PathBuf {
    let ring = "13000 -1500 17000 -1500 17000 -6500 13000 -6500 13000 -1500";
    let wiring = format!(
        "(wiring (wire (path F.Cu 200 {ring}) (net Across) (type {kind})) \
           (wire (path B.Cu 200 {ring}) (net Across) (type {kind})))"
    );
    let board = std::fs::read_to_string(fixture("two-nets.dsn"))
        .expect("fixture")
        .replace("(wiring)", &wiring);
    let file = folder.join(format!("ringed-{kind}.dsn"));
    std::fs::write(&file, board).expect("written");
    file
}

fn route_ringed(kind: &str, reroute_existing: bool) -> (RouteReport, String) {
    let folder = tempfile::tempdir().expect("temp dir");
    let session = folder.path().join("ringed.ses");
    let job = RouteJob {
        input: ringed_board(folder.path(), kind),
        reroute_existing,
        ..job("two-nets.dsn", &session)
    };
    let report = route(&job, &Silent, &CancelToken::new()).expect("routable");
    (report, std::fs::read_to_string(&session).expect("stored"))
}

#[test]
fn the_boards_unprotected_wiring_stays_as_it_is_unless_asked_to_route_it_again() {
    let (kept, _) = route_ringed("route", false);
    assert!(
        kept.unrouted.iter().any(|u| u.net == "Down"),
        "{:?}",
        kept.unrouted
    );
    let (rerouted, session) = route_ringed("route", true);
    assert!(rerouted.unrouted.is_empty(), "{:?}", rerouted.unrouted);
    // A corner of the ring, in the session's steps of 0.1 µm.
    assert!(
        !session.contains("170000 -65000"),
        "the ring is gone: {session}"
    );
}

#[test]
fn protected_wiring_stays_even_when_asked_to_route_the_rest_again() {
    let (report, _) = route_ringed("protect", true);
    assert!(
        report.unrouted.iter().any(|u| u.net == "Down"),
        "{:?}",
        report.unrouted
    );
}
