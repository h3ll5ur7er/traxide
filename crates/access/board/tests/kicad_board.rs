//! Boards read from KiCad's own files, compared with what KiCad states in them
//! and with KiCad's own DSN export of the same boards, from KiCad 10 and 9.

use std::collections::BTreeMap;
use std::path::Path;
use traxide_board_access::load_design;
use traxide_contracts::geometry::{Point, Rect};
use traxide_contracts::{Design, LayerId};

fn fixture(name: &str) -> String {
    format!(
        "{}/tests/fixtures/kicad10/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn board(name: &str) -> Design {
    load_design(Path::new(&fixture(name))).expect("KiCad's board is valid")
}

const MM: i32 = 1_000_000;

#[test]
fn copper_layers_are_read_from_top_to_bottom() {
    let names: Vec<String> = board("crossing.kicad_pcb")
        .layers
        .into_iter()
        .map(|l| l.name)
        .collect();
    assert_eq!(names, ["F.Cu", "B.Cu"]);
}

#[test]
fn nets_join_the_pads_that_name_them() {
    let design = board("crossing.kicad_pcb");
    assert_eq!(pads_by_net(&design)["ACROSS"].len(), 2);
    assert_eq!(pads_by_net(&design)["DOWN"].len(), 2);
}

#[test]
fn the_outline_is_the_loop_of_edge_cuts() {
    let design = board("crossing.kicad_pcb");
    assert_eq!(
        design.outline.bounds(),
        Rect {
            min: Point::new(0, 0),
            max: Point::new(30 * MM, 30 * MM)
        }
    );
}

#[test]
fn rules_come_from_the_project() {
    let rules = board("crossing.kicad_pcb").rules;
    let default = &rules.classes[rules.default_class.index()];
    assert_eq!(default.name, "Default");
    assert_eq!((default.track_width, default.clearance), (MM / 5, MM / 5));
    assert_eq!(rules.edge_clearance, MM / 2);
}

#[test]
fn a_board_without_its_project_follows_kicads_defaults() {
    let folder = tempfile::tempdir().expect("temporary folder");
    let alone = folder.path().join("crossing.kicad_pcb");
    std::fs::copy(fixture("crossing.kicad_pcb"), &alone).expect("copied");
    let design = load_design(&alone).expect("valid without its project");
    assert_eq!(design.rules, board("crossing.kicad_pcb").rules);
}

/// A copy of the crossing board in `folder`, with `custom_rules` beside it.
fn crossing_with_custom_rules(folder: &Path, custom_rules: &str) -> std::path::PathBuf {
    for extension in ["kicad_pcb", "kicad_pro"] {
        std::fs::copy(
            fixture(&format!("crossing.{extension}")),
            folder.join(format!("crossing.{extension}")),
        )
        .expect("copied");
    }
    std::fs::write(folder.join("crossing.kicad_dru"), custom_rules).expect("written");
    folder.join("crossing.kicad_pcb")
}

#[test]
fn custom_rules_come_from_the_rules_file_beside_the_board() {
    let folder = tempfile::tempdir().expect("temporary folder");
    let board = crossing_with_custom_rules(
        folder.path(),
        "(version 1)\n(rule wide (constraint clearance (min 1mm)) (condition \"A.NetName == 'DOWN'\"))",
    );
    let custom = load_design(&board).expect("valid").rules.custom;
    assert_eq!(custom.len(), 1);
    assert_eq!(custom[0].name, "wide");
    assert_eq!(custom[0].limits.min, Some(MM));
}

#[test]
fn a_broken_custom_rule_is_left_out_with_a_warning_naming_its_line() {
    let folder = tempfile::tempdir().expect("temporary folder");
    let board = crossing_with_custom_rules(
        folder.path(),
        "(version 1)\n(rule r\n  (constraint clearance (min 1)))\n(rule wide (constraint clearance (min 1mm)))",
    );
    let design = load_design(&board).expect("routable with the rules it can read");
    let names: Vec<&str> = design
        .rules
        .custom
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(names, ["wide"]);
    assert_eq!(design.warnings.len(), 1, "{:?}", design.warnings);
    assert!(
        design.warnings[0].contains("custom rule r") && design.warnings[0].contains("line 3"),
        "{:?}",
        design.warnings
    );
}

#[test]
fn the_boards_own_tracks_and_vias_are_existing_copper() {
    let design = board("crossing.routed.kicad_pcb");
    assert_eq!(design.existing.tracks.len(), 4);
    assert_eq!(design.existing.vias.len(), 2);
    let via = &design.existing.vias[0];
    assert_eq!(design.net(via.net).name, "DOWN");
    let kind = design.via_kind(via.kind);
    let layers: Vec<LayerId> = kind.copper.iter().map(|c| c.layer).collect();
    assert_eq!(layers, [LayerId::from_index(0), LayerId::from_index(1)]);
    assert_eq!(width(kind.copper[0].shape.bounds()), 6 * MM / 10);
}

#[test]
fn a_board_file_agrees_with_kicads_dsn_export_of_it() {
    let boards = [
        "kicad10/placements",
        "kicad10/crossing",
        "kicad10/matrix-3x4",
        "kicad10/keyboard-tkl",
        "kicad9/placements",
        "kicad9/crossing",
        "kicad9/matrix-3x4",
    ];
    for recipe in boards {
        let read = |extension: &str| {
            let path = format!(
                "{}/tests/fixtures/{recipe}.{extension}",
                env!("CARGO_MANIFEST_DIR")
            );
            load_design(Path::new(&path)).expect("KiCad's files are valid")
        };
        let (from_board, from_dsn) = (read("kicad_pcb"), read("dsn"));
        let layers = |d: &Design| d.layers.clone();
        assert_eq!(layers(&from_board), layers(&from_dsn), "{recipe}");
        assert_eq!(pads_by_net(&from_board), pads_by_net(&from_dsn), "{recipe}");
        for pad in &from_dsn.pads {
            let same = from_board
                .pads
                .iter()
                .find(|p| p.name == pad.name)
                .unwrap_or_else(|| panic!("{recipe}: {} is missing", pad.name));
            let distance = same.position - pad.position;
            assert!(
                distance.x.abs() <= from_dsn.resolution && distance.y.abs() <= from_dsn.resolution,
                "{recipe}: {} at {:?}, the DSN has {:?}",
                pad.name,
                same.position,
                pad.position
            );
        }
        let rules = |d: &Design| {
            let class = &d.rules.classes[d.rules.default_class.index()];
            let via = &d.via_kind(class.via_kinds[0]).copper[0].shape;
            (
                class.track_width,
                class.clearance,
                d.rules.edge_clearance,
                width(via.bounds()),
            )
        };
        assert_eq!(rules(&from_board), rules(&from_dsn), "{recipe}");
        let corners = |d: &Design| {
            let mut corners: Vec<Point> = d.outline.edges().map(|e| e.a).collect();
            corners.sort();
            corners
        };
        let near = |p: Point, q: Point| {
            (p.x - q.x).abs() <= from_dsn.resolution && (p.y - q.y).abs() <= from_dsn.resolution
        };
        let (board_corners, dsn_corners) = (corners(&from_board), corners(&from_dsn));
        // KiCad's DSN export writes the outline in whole micrometres.
        let near_micrometre =
            |p: Point, q: Point| (p.x - q.x).abs() <= 1_000 && (p.y - q.y).abs() <= 1_000;
        assert!(
            board_corners.len() == dsn_corners.len()
                && board_corners
                    .iter()
                    .zip(&dsn_corners)
                    .all(|(&a, &b)| near_micrometre(a, b)),
            "{recipe}: the outline's corners are {board_corners:?}, the DSN's {dsn_corners:?}"
        );
        let keepouts = |d: &Design| {
            // KiCad's DSN export lists a keepout once on each layer.
            let mut keepouts: Vec<(Option<LayerId>, Rect)> = d
                .keepouts
                .iter()
                .flat_map(|k| {
                    let layers: Vec<LayerId> = match k.layer {
                        Some(layer) => vec![layer],
                        None => d.layer_ids().collect(),
                    };
                    layers.into_iter().map(|l| (Some(l), k.shape.bounds()))
                })
                .collect();
            keepouts.sort_by_key(|(layer, r)| (*layer, r.min, r.max));
            keepouts
        };
        let (board_keepouts, dsn_keepouts) = (keepouts(&from_board), keepouts(&from_dsn));
        assert_eq!(board_keepouts.len(), dsn_keepouts.len(), "{recipe}");
        for ((layer, a), (dsn_layer, b)) in board_keepouts.iter().zip(&dsn_keepouts) {
            assert!(
                layer == dsn_layer && near(a.min, b.min) && near(a.max, b.max),
                "{recipe}: a keepout {a:?} on {layer:?}, the DSN has {b:?} on {dsn_layer:?}"
            );
        }
    }
}

fn pads_by_net(design: &Design) -> BTreeMap<String, Vec<String>> {
    let mut nets = BTreeMap::new();
    for net in &design.nets {
        let mut pads: Vec<String> = net
            .pads
            .iter()
            .map(|&p| design.pad(p).name.clone())
            .collect();
        pads.sort();
        if !pads.is_empty() {
            nets.insert(net.name.clone(), pads);
        }
    }
    nets
}

fn width(rect: Rect) -> i32 {
    rect.max.x - rect.min.x
}

mod storing {
    use super::*;
    use traxide_board_access::{BoardError, store_routes};
    use traxide_contracts::{NetId, Routes, Track, Via};

    fn routes(design: &Design) -> Routes {
        let net = |name: &str| {
            NetId::from_index(
                design
                    .nets
                    .iter()
                    .position(|n| n.name == name)
                    .expect("net"),
            )
        };
        let default = &design.rules.classes[design.rules.default_class.index()];
        Routes {
            tracks: vec![Track {
                net: net("DOWN"),
                layer: LayerId::from_index(1),
                width: default.track_width,
                path: vec![
                    Point::new(15 * MM, 5 * MM),
                    Point::new(15 * MM, 10 * MM),
                    Point::new(16 * MM + 1, 11 * MM),
                ],
            }],
            vias: vec![Via {
                net: net("DOWN"),
                kind: default.via_kinds[0],
                position: Point::new(15 * MM, 5 * MM),
            }],
        }
    }

    #[test]
    fn a_design_that_routes_the_unlocked_wiring_again_is_stored_without_it() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let board = std::fs::read_to_string(fixture("crossing.kicad_pcb")).expect("fixture");
        let end = board
            .trim_end()
            .rfind(')')
            .expect("the board's closing parenthesis");
        let wiring = "\t(segment (start 1 1) (end 2 1) (width 0.2) (layer \"F.Cu\") (locked yes) (net \"ACROSS\"))\n\
                      \t(segment (start 1 2) (end 2 2) (width 0.2) (layer \"F.Cu\") (net \"ACROSS\"))\n";
        let input = folder.path().join("wired.kicad_pcb");
        std::fs::write(&input, format!("{}{wiring})\n", &board[..end])).expect("written");
        let mut design = load_design(&input).expect("a valid board");
        assert_eq!(design.existing.tracks.len(), 2);
        design.existing = design.locked.clone();
        let output = folder.path().join("routed.kicad_pcb");
        store_routes(&output, &input, &design, &Routes::default()).expect("stored");
        let stored = std::fs::read_to_string(&output).expect("stored");
        assert!(stored.contains("(locked yes)"), "{stored}");
        assert!(!stored.contains("(start 1 2)"), "{stored}");
    }

    #[test]
    fn routes_stored_into_a_board_file_are_its_copper_when_read_again() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let input = Path::new(&fixture("crossing.kicad_pcb")).to_owned();
        let output = folder.path().join("routed.kicad_pcb");
        let design = board("crossing.kicad_pcb");
        let routes = routes(&design);
        store_routes(&output, &input, &design, &routes).expect("stored");
        let routed = load_design(&output).expect("a valid board");
        assert_eq!(routed.existing.vias, routes.vias);
        let segments: Vec<Vec<Point>> = routed
            .existing
            .tracks
            .iter()
            .map(|t| t.path.clone())
            .collect();
        assert_eq!(
            segments,
            routes.tracks[0]
                .path
                .windows(2)
                .map(<[Point]>::to_vec)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_routed_board_keeps_the_custom_rules_of_the_board() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let rules = "(version 1)\n(rule wide (constraint clearance (min 1mm)))";
        let input = crossing_with_custom_rules(folder.path(), rules);
        let output = folder.path().join("routed.kicad_pcb");
        let design = load_design(&input).expect("valid");
        store_routes(&output, &input, &design, &routes(&design)).expect("stored");
        let copied = std::fs::read_to_string(folder.path().join("routed.kicad_dru"));
        assert_eq!(copied.expect("copied beside the routed board"), rules);
    }

    #[test]
    fn every_byte_of_the_board_stays_as_it_was() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let input = Path::new(&fixture("crossing.kicad_pcb")).to_owned();
        let output = folder.path().join("routed.kicad_pcb");
        let design = board("crossing.kicad_pcb");
        store_routes(&output, &input, &design, &routes(&design)).expect("stored");
        let original = std::fs::read_to_string(&input).expect("readable");
        let stored = std::fs::read_to_string(&output).expect("readable");
        let kept = original.trim_end().strip_suffix(')').expect("closes");
        assert!(stored.starts_with(kept));
    }

    #[test]
    fn a_board_that_changed_since_it_was_read_is_not_stored_into() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let input = folder.path().join("crossing.kicad_pcb");
        std::fs::copy(fixture("crossing.kicad_pcb"), &input).expect("copied");
        let design = board("crossing.kicad_pcb");
        let changed = std::fs::read_to_string(&input)
            .expect("readable")
            .replace("(at 25 15)", "(at 26 15)");
        std::fs::write(&input, changed).expect("written");
        let output = folder.path().join("routed.kicad_pcb");
        let error = store_routes(&output, &input, &design, &routes(&design)).unwrap_err();
        assert!(matches!(error, BoardError::Changed { .. }), "{error}");
        assert!(!output.exists());
    }
}

#[test]
fn copper_keeps_a_little_more_from_the_edges_of_a_kicad_board_whose_arcs_are_read_as_chords() {
    let design = board("matrix-3x4.kicad_pcb");
    // The reader's chords lie within 2 µm of the arcs they stand for.
    assert_eq!(design.outline_tolerance, 2_000);
}
