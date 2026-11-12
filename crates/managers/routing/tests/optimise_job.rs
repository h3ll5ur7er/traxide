use std::path::PathBuf;
use traxide_events::{CancelToken, Silent};
use traxide_routing::{OptimiseJob, optimise};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(format!(
        "{}/../../../fixtures/boards/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
}

/// The crossing board with DOWN's pads, at (15, 5) and (15, 25) mm, joined
/// by a detour of separate segments as KiCad stores wiring, and the
/// board's project beside it, in a folder of its own.
fn detoured() -> (tempfile::TempDir, PathBuf) {
    let folder = tempfile::tempdir().expect("temp dir");
    let board = std::fs::read_to_string(fixture("crossing.kicad_pcb")).expect("fixture");
    let end = board
        .trim_end()
        .rfind(')')
        .expect("the board's closing parenthesis");
    let detour = [(15, 5), (20, 10), (20, 20), (15, 25)];
    let segments: String = detour
        .windows(2)
        .map(|w| {
            format!(
                "\t(segment (start {} {}) (end {} {}) (width 0.2) (layer \"F.Cu\") (net \"DOWN\"))\n",
                w[0].0, w[0].1, w[1].0, w[1].1
            )
        })
        .collect();
    let input = folder.path().join("crossing.kicad_pcb");
    std::fs::write(&input, format!("{}{segments})\n", &board[..end])).expect("written");
    std::fs::copy(
        fixture("crossing.kicad_pro"),
        folder.path().join("crossing.kicad_pro"),
    )
    .expect("copied");
    (folder, input)
}

#[test]
fn a_boards_own_detour_is_pulled_tight_into_a_copy_beside_it() {
    let (folder, input) = detoured();
    let job = OptimiseJob {
        input,
        output: None,
    };
    let report = optimise(&job, &Silent, &CancelToken::new()).expect("optimised");
    assert_eq!(
        report.output,
        folder.path().join("crossing.optimised.kicad_pcb")
    );
    assert!(report.stored, "{report:?}");
    assert!(report.length_after < report.length_before, "{report:?}");
    assert!(
        report
            .verdict
            .violations
            .iter()
            .all(|v| matches!(v, traxide_contracts::Violation::Unconnected { .. }))
    );
    let optimised = traxide_board_access::load_design(&report.output).expect("stored");
    let length: i64 = (optimised.existing.tracks.iter())
        .flat_map(|t| t.path.windows(2))
        .map(|w| w[0].distance(w[1]))
        .sum();
    assert_eq!(length, 20_000_000, "straight from (15, 5) to (15, 25)");
}

#[test]
fn the_tracks_of_a_differential_pair_are_left_as_they_were() {
    let folder = tempfile::tempdir().expect("temp dir");
    let board = std::fs::read_to_string(fixture("pair.kicad_pcb")).expect("fixture");
    let end = board
        .trim_end()
        .rfind(')')
        .expect("the board's closing parenthesis");
    let detour = [(5.0, 14.5), (10.0, 10.0), (20.0, 10.0), (25.0, 14.5)];
    let segments: String = detour
        .windows(2)
        .map(|w| {
            format!(
                "\t(segment (start {} {}) (end {} {}) (width 0.2) (layer \"F.Cu\") (net \"USB_D_P\"))\n",
                w[0].0, w[0].1, w[1].0, w[1].1
            )
        })
        .collect();
    let input = folder.path().join("pair.kicad_pcb");
    std::fs::write(&input, format!("{}{segments})\n", &board[..end])).expect("written");
    std::fs::copy(
        fixture("pair.kicad_pro"),
        folder.path().join("pair.kicad_pro"),
    )
    .expect("copied");
    let job = OptimiseJob {
        input,
        output: None,
    };
    let report = optimise(&job, &Silent, &CancelToken::new()).expect("optimised");
    assert!(!report.stored, "{report:?}");
    assert_eq!(report.length_after, report.length_before);
}
