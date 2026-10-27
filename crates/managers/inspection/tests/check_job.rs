use std::path::PathBuf;
use traxide_inspection::{CheckJob, check};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(format!(
        "{}/../../../fixtures/boards/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
}

#[test]
fn a_board_without_routes_has_every_net_unconnected() {
    let report = check(&CheckJob {
        board: fixture("two-nets.dsn"),
        session: None,
        drc_report: None,
    })
    .expect("readable");
    assert_eq!((report.nets, report.incomplete_nets), (2, 2));
    assert_eq!(report.unconnected_pads, 2);
    assert!(report.violations.is_empty());
}

#[test]
fn a_board_with_its_routes_is_complete_and_clean() {
    let session = tempfile::tempdir()
        .expect("temp dir")
        .keep()
        .join("routed.ses");
    std::fs::write(
        &session,
        "(session routed.ses (base_design two-nets.dsn)
           (routes (resolution um 10) (parser)
             (network_out
               (net Across (wire (path F.Cu 2000 60000 -100000 240000 -100000)))
               (net Down
                 (wire (path F.Cu 2000 150000 -50000 150000 -80000))
                 (via \"Via[0-1]_600:300_um\" 150000 -80000)
                 (wire (path B.Cu 2000 150000 -80000 150000 -120000))
                 (via \"Via[0-1]_600:300_um\" 150000 -120000)
                 (wire (path F.Cu 2000 150000 -120000 150000 -150000))))))",
    )
    .expect("writable");
    let report = check(&CheckJob {
        board: fixture("two-nets.dsn"),
        session: Some(session),
        drc_report: None,
    })
    .expect("readable");
    assert_eq!(report.incomplete_nets, 0, "{report:?}");
    assert!(report.violations.is_empty(), "{report:?}");
    assert_eq!((report.tracks, report.vias), (4, 2));
}

#[test]
fn a_session_is_judged_with_the_locked_wiring_of_the_board_which_kicads_import_keeps() {
    let folder = tempfile::tempdir().expect("temp dir");
    let board = folder.path().join("locked.dsn");
    let design = std::fs::read_to_string(fixture("two-nets.dsn")).expect("fixture");
    std::fs::write(
        &board,
        design.replace(
            "(wiring)",
            "(wiring (wire (path F.Cu 200 6000 -10000 24000 -10000) (net Across) (type fix)))",
        ),
    )
    .expect("writable");
    let session = folder.path().join("locked.ses");
    std::fs::write(
        &session,
        "(session locked.ses (base_design locked.dsn)
           (routes (resolution um 10) (parser)
             (network_out
               (net Down
                 (wire (path F.Cu 2000 150000 -50000 150000 -80000))
                 (via \"Via[0-1]_600:300_um\" 150000 -80000)
                 (wire (path B.Cu 2000 150000 -80000 150000 -120000))
                 (via \"Via[0-1]_600:300_um\" 150000 -120000)
                 (wire (path F.Cu 2000 150000 -120000 150000 -150000))))))",
    )
    .expect("writable");
    let report = check(&CheckJob {
        board,
        session: Some(session),
        drc_report: None,
    })
    .expect("readable");
    assert_eq!(report.incomplete_nets, 0, "{report:?}");
}

#[test]
fn copper_that_breaks_a_rule_is_reported_by_kind() {
    let report = check(&CheckJob {
        board: fixture("existing-violation.dsn"),
        session: None,
        drc_report: None,
    })
    .expect("readable");
    // Its stray wire also ends in nothing at both ends.
    assert_eq!(
        report.violations_by_kind(),
        [("clearance", 1), ("dangling", 2)]
    );
}

#[test]
fn the_check_writes_a_drc_report_in_kicads_format_when_asked() {
    let folder = tempfile::tempdir().expect("temp dir");
    let drc = folder.path().join("report.json");
    check(&CheckJob {
        board: fixture("existing-violation.dsn"),
        session: None,
        drc_report: Some(drc.clone()),
    })
    .expect("readable");
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(drc).expect("written")).expect("JSON");
    assert_eq!(report["source"], "existing-violation.dsn");
    assert_eq!(report["violations"][0]["type"], "clearance");
    assert_eq!(report["violations"][1]["type"], "track_dangling");
    assert_eq!(report["violations"][1]["severity"], "warning");
    assert_eq!(report["violations"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        report["unconnected_items"].as_array().map(Vec::len),
        Some(2),
        "one pad of each net apart"
    );
}

#[test]
fn names_pads_of_different_nets_closer_than_their_clearance() {
    // R3-1 of net Short overlaps R1-2 of net Across.
    let report = check(&CheckJob {
        board: fixture("pads-too-close.dsn"),
        session: None,
        drc_report: None,
    })
    .expect("readable");
    assert_eq!(
        report.warnings,
        ["the pads R1-2 (net Across) and R3-1 (net Short) are closer than their clearance"]
    );
}
