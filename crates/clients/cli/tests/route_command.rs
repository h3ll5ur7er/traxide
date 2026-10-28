//! End to end: the `traxide` binary routes a board, and what it writes is read
//! back and checked independently.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use traxide_rules::Purpose;

fn board(name: &str) -> PathBuf {
    PathBuf::from(format!(
        "{}/../../../fixtures/boards/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
}

fn traxide(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_traxide"))
        .args(args)
        .output()
        .expect("the binary runs")
}

fn copy_to(folder: &Path, name: &str) -> PathBuf {
    let copy = folder.join(name);
    std::fs::copy(board(name), &copy).expect("copied");
    copy
}

#[test]
fn routes_a_board_and_writes_a_session_next_to_it() {
    let folder = tempfile::tempdir().expect("temp dir");
    let input = copy_to(folder.path(), "two-nets.dsn");
    let run = traxide(&["route", input.to_str().expect("utf-8"), "--quiet"]);
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(stdout.contains("Routed 2 of 2 connections"), "{stdout}");

    let design = traxide_board_access::load_design(&input).expect("valid board");
    let routes = traxide_board_access::load_routes(&input.with_extension("ses"), &design)
        .expect("valid session");
    let rules = traxide_rules::compile(&design, Purpose::Verification);
    let verdict = traxide_verification::verify(&design, &rules, &routes);
    assert!(verdict.is_clean(), "{verdict:?}");
}

#[test]
fn routes_only_the_nets_named() {
    let folder = tempfile::tempdir().expect("temp dir");
    let input = copy_to(folder.path(), "two-nets.dsn");
    let input = input.to_str().expect("utf-8");
    let run = traxide(&["route", input, "--net", "Down", "--quiet"]);
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(run.status.success(), "{stdout}");
    assert!(stdout.contains("Routed 1 of 1 connections"), "{stdout}");
    let run = traxide(&["route", input, "--net", "Dwon", "--quiet"]);
    assert!(!run.status.success());
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("no net named Dwon"),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn a_quiet_route_of_a_complete_board_prints_nothing_but_its_summary() {
    let folder = tempfile::tempdir().expect("temp dir");
    let input = copy_to(folder.path(), "two-nets.dsn");
    let run = traxide(&["route", input.to_str().expect("utf-8"), "--quiet"]);
    assert_eq!(String::from_utf8_lossy(&run.stderr), "");
    assert_eq!(String::from_utf8_lossy(&run.stdout).lines().count(), 1);
}

#[test]
fn writes_the_same_session_on_any_number_of_threads() {
    let folder = tempfile::tempdir().expect("temp dir");
    // Exported by KiCad 10 from the matrix-3x4 recipe of e2e/kicad/boards.py.
    let input = copy_to(folder.path(), "matrix-3x4.dsn");
    let session_on = |threads: &str| {
        // The same file name each time, as a session names itself after its file.
        let output = folder.path().join(threads).join("routed.ses");
        std::fs::create_dir(output.parent().expect("in a folder")).expect("new folder");
        let run = traxide(&[
            "route",
            input.to_str().expect("utf-8"),
            "-o",
            output.to_str().expect("utf-8"),
            "--threads",
            threads,
            "--quiet",
        ]);
        assert!(run.status.success(), "{run:?}");
        std::fs::read(output).expect("written")
    };
    let on_one = session_on("1");
    assert_eq!(session_on("4"), on_one);
    assert_eq!(session_on("16"), on_one);
}

#[test]
fn an_unknown_profile_fails_with_the_known_ones() {
    let folder = tempfile::tempdir().expect("temp dir");
    let input = copy_to(folder.path(), "two-nets.dsn");
    let run = traxide(&[
        "route",
        input.to_str().expect("utf-8"),
        "--profile",
        "turbo",
    ]);
    assert_eq!(run.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("no profile named turbo; there are default")
    );
}

#[test]
fn a_missing_board_fails_and_says_which() {
    let run = traxide(&["route", "no-such-board.dsn"]);
    assert_eq!(run.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&run.stderr).contains("no-such-board.dsn"));
}

#[test]
fn check_reports_how_complete_a_board_is_and_exits_accordingly() {
    let unrouted = traxide(&["check", board("two-nets.dsn").to_str().expect("utf-8")]);
    let stdout = String::from_utf8_lossy(&unrouted.stdout);
    assert_eq!(unrouted.status.code(), Some(2), "{stdout}");
    assert!(stdout.contains("0 of 2 nets complete"), "{stdout}");

    let folder = tempfile::tempdir().expect("temp dir");
    let input = copy_to(folder.path(), "two-nets.dsn");
    let routed = traxide(&["route", input.to_str().expect("utf-8"), "--quiet"]);
    assert!(routed.status.success());
    let session = input.with_extension("ses");
    let checked = traxide(&[
        "check",
        input.to_str().expect("utf-8"),
        session.to_str().expect("utf-8"),
        "--json",
    ]);
    assert_eq!(checked.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&checked.stdout).expect("a JSON report");
    assert_eq!(report["incomplete_nets"], 0);
    assert_eq!(report["violations"], serde_json::json!({}));
}

/// The DRC report at `path`, which must parse and hold every field KiCad's
/// schema requires of a report.
fn drc_report(path: &Path) -> serde_json::Value {
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("written")).expect("JSON");
    for key in [
        "$schema",
        "source",
        "date",
        "kicad_version",
        "coordinate_units",
    ] {
        assert!(report[key].is_string(), "{key} in {report}");
    }
    for key in ["violations", "unconnected_items", "schematic_parity"] {
        assert!(report[key].is_array(), "{key} in {report}");
    }
    report
}

#[test]
fn check_writes_a_drc_report_in_kicads_format_with_drc() {
    let folder = tempfile::tempdir().expect("temp dir");
    let drc = folder.path().join("report.json");
    let checked = traxide(&[
        "check",
        board("existing-violation.dsn").to_str().expect("utf-8"),
        "--drc",
        drc.to_str().expect("utf-8"),
    ]);
    assert_eq!(checked.status.code(), Some(3), "{checked:?}");
    let report = drc_report(&drc);
    assert_eq!(report["violations"][0]["type"], "clearance");
    assert_eq!(
        report["unconnected_items"].as_array().map(Vec::len),
        Some(2)
    );
}

#[test]
fn route_writes_a_drc_report_of_the_stored_routes_with_drc() {
    let folder = tempfile::tempdir().expect("temp dir");
    let input = copy_to(folder.path(), "two-nets.dsn");
    let drc = folder.path().join("two-nets.drc.json");
    let routed = traxide(&[
        "route",
        input.to_str().expect("utf-8"),
        "--quiet",
        "--drc",
        drc.to_str().expect("utf-8"),
    ]);
    assert!(routed.status.success(), "{routed:?}");
    let report = drc_report(&drc);
    assert_eq!(report["source"], "two-nets.ses");
    assert_eq!(report["violations"], serde_json::json!([]));
    assert_eq!(report["unconnected_items"], serde_json::json!([]));
}

#[test]
fn routing_the_board_open_in_kicad_explains_how_to_enable_kicads_api_when_it_is_off() {
    let folder = tempfile::tempdir().expect("temp dir");
    let run = Command::new(env!("CARGO_BIN_EXE_traxide"))
        .args(["route", "--kicad"])
        .env("KICAD_API_SOCKET", folder.path().join("api.sock"))
        .output()
        .expect("the binary runs");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("cannot reach KiCad"), "{stderr}");
    assert!(
        stderr.contains("Preferences → Preferences… → Plugins → Enable KiCad API"),
        "{stderr}"
    );
}

#[test]
fn a_route_names_its_board_or_the_one_open_in_kicad() {
    let run = traxide(&["route"]);
    assert_eq!(run.status.code(), Some(2), "clap's usage error");
    let board = board("two-nets.dsn");
    let both = traxide(&["route", "--kicad", board.to_str().expect("utf-8")]);
    assert_eq!(both.status.code(), Some(2), "clap's usage error");
}

#[test]
fn only_kicad_has_a_selection_to_route_the_nets_of() {
    let board = board("two-nets.dsn");
    let run = traxide(&["route", board.to_str().expect("utf-8"), "--selected"]);
    assert_eq!(run.status.code(), Some(2), "clap's usage error");
}

/// KiCad starts a plugin's entry point with no arguments, its API's socket in
/// the environment.
#[test]
fn started_by_kicad_without_arguments_it_routes_the_board_open_in_kicad() {
    let folder = tempfile::tempdir().expect("temp dir");
    let run = Command::new(env!("CARGO_BIN_EXE_traxide"))
        .env("KICAD_API_SOCKET", folder.path().join("api.sock"))
        .output()
        .expect("the binary runs");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("cannot reach KiCad"), "{stderr}");
}

#[test]
fn started_without_arguments_outside_kicad_it_shows_how_to_use_it() {
    let run = Command::new(env!("CARGO_BIN_EXE_traxide"))
        .env_remove("KICAD_API_SOCKET")
        .output()
        .expect("the binary runs");
    assert_eq!(run.status.code(), Some(2), "clap's usage error");
    assert!(String::from_utf8_lossy(&run.stderr).contains("Usage"));
}
