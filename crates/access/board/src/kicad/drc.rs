//! Verdicts as DRC reports in the JSON schema `kicad-cli pcb drc --format json`
//! writes (https://schemas.kicad.org/drc.v1.json), so that tools that read
//! KiCad's reports read traxide's too.

use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};
use traxide_contracts::geometry::Point;
use traxide_contracts::{CopperItem, Design, LayerId, NetId, Routes, Verdict, Violation};

/// The report of `verdict`, found on `routes` of `design` in the file named
/// `source`, written at `written`.
pub(crate) fn report(
    source: &str,
    written: SystemTime,
    design: &Design,
    routes: &Routes,
    verdict: &Verdict,
) -> Value {
    let item =
        |copper: CopperItem, layer: Option<LayerId>| affected_item(design, routes, copper, layer);
    let mut violations = Vec::new();
    let mut unconnected_items = Vec::new();
    for violation in &verdict.violations {
        match violation {
            Violation::Clearance {
                layer,
                a,
                b,
                required,
            } => violations.push(entry(
                "clearance",
                &format!(
                    "Clearance violation (clearance {:.4} mm)",
                    millimetres(*required)
                ),
                vec![item(*a, Some(*layer)), item(*b, Some(*layer))],
            )),
            // KiCad's default severity for holes too close is a warning.
            Violation::HoleToHole { a, b, required } => violations.push(json!({
                "type": "hole_to_hole",
                "description": format!(
                    "Drilled hole too close to other hole (min {:.4} mm)",
                    millimetres(*required)
                ),
                "severity": "warning",
                "items": [item(*a, None), item(*b, None)],
            })),
            Violation::TrackWidth {
                item: copper,
                width,
                min,
                max,
            } => violations.push(entry(
                "track_width",
                &format!("Track width ({})", out_of_limits(*width, *min, *max)),
                vec![item(*copper, None)],
            )),
            Violation::ViaDiameter {
                item: copper,
                diameter,
                min,
                max,
            } => violations.push(entry(
                "via_diameter",
                &format!("Via diameter ({})", out_of_limits(*diameter, *min, *max)),
                vec![item(*copper, None)],
            )),
            Violation::HoleSize {
                item: copper,
                drill,
                min,
                max,
            } => violations.push(entry(
                "drill_out_of_range",
                &format!(
                    "Hole size out of range ({})",
                    out_of_limits(*drill, *min, *max)
                ),
                vec![item(*copper, None)],
            )),
            Violation::Edge {
                layer,
                item: copper,
            } => violations.push(entry(
                "copper_edge_clearance",
                "Board edge clearance violation",
                vec![item(*copper, Some(*layer))],
            )),
            Violation::Keepout {
                layer,
                item: copper,
            } => violations.push(entry(
                "items_not_allowed",
                "Items not allowed",
                vec![item(*copper, Some(*layer))],
            )),
            // KiCad's default severity for dangling copper is a warning.
            Violation::Dangling {
                layer,
                item: copper,
                ..
            } => {
                let (kind, description) = match copper {
                    CopperItem::Via(_) => (
                        "via_dangling",
                        "Via is not connected or connected on only one layer",
                    ),
                    _ => ("track_dangling", "Track has unconnected end"),
                };
                violations.push(json!({
                    "type": kind,
                    "description": description,
                    "severity": "warning",
                    "items": [item(*copper, Some(*layer))],
                }));
            }
            Violation::Unconnected { net, pads } => {
                // traxide finds the pads copper leaves apart from the net's
                // first pad; KiCad lists missing connections in pairs.
                let Some(&first) = design.net(*net).pads.first() else {
                    continue;
                };
                for &pad in pads {
                    unconnected_items.push(entry(
                        "unconnected_items",
                        "Missing connection between items",
                        vec![
                            item(CopperItem::Pad(first), None),
                            item(CopperItem::Pad(pad), None),
                        ],
                    ));
                }
            }
        }
    }
    json!({
        "$schema": "https://schemas.kicad.org/drc.v1.json",
        "source": source,
        "date": date(written),
        "kicad_version": format!("traxide {}", env!("CARGO_PKG_VERSION")),
        "coordinate_units": "mm",
        "violations": violations,
        "unconnected_items": unconnected_items,
        "schematic_parity": [],
    })
}

/// The limit `value` breaks and the value itself, as KiCad words them.
fn out_of_limits(value: i32, min: Option<i32>, max: Option<i32>) -> String {
    let limit = match (min, max) {
        (Some(min), _) if value < min => format!("min {:.4} mm; ", millimetres(min)),
        (_, Some(max)) => format!("max {:.4} mm; ", millimetres(max)),
        _ => String::new(),
    };
    format!("{limit}actual {:.4} mm", millimetres(value))
}

/// Every violation traxide finds breaks a rule: KiCad's `error`.
fn entry(kind: &str, description: &str, items: Vec<Value>) -> Value {
    json!({
        "type": kind,
        "description": description,
        "severity": "error",
        "items": items,
    })
}

/// A piece of copper as KiCad describes it, on `layer` where it breaks a rule
/// there; without the UUID KiCad gives each item, which designs do not carry.
fn affected_item(
    design: &Design,
    routes: &Routes,
    copper: CopperItem,
    layer: Option<LayerId>,
) -> Value {
    let layer_name = |layer: LayerId| design.layers[layer.index()].name.as_str();
    let (description, position) = match copper {
        CopperItem::Pad(pad) => {
            let pad = design.pad(pad);
            let on = layer.map_or(String::new(), |l| format!(" on {}", layer_name(l)));
            (
                format!("Pad {} [{}]{on}", pad.name, net_name(design, pad.net)),
                pad.position,
            )
        }
        CopperItem::Track(index) => {
            let track = &routes.tracks[index];
            let length: i64 = track.path.windows(2).map(|p| p[0].distance(p[1])).sum();
            (
                format!(
                    "Track [{}] on {}, length {:.4} mm",
                    net_name(design, Some(track.net)),
                    layer_name(track.layer),
                    length as f64 / 1e6
                ),
                track.path[0],
            )
        }
        CopperItem::Via(index) => {
            let via = &routes.vias[index];
            let copper = &design.via_kind(via.kind).copper;
            let (top, bottom) = (copper[0].layer, copper[copper.len() - 1].layer);
            (
                format!(
                    "Via [{}] on {} - {}",
                    net_name(design, Some(via.net)),
                    layer_name(top),
                    layer_name(bottom)
                ),
                via.position,
            )
        }
    };
    json!({"description": description, "pos": position_in_millimetres(position)})
}

fn net_name(design: &Design, net: Option<NetId>) -> &str {
    net.map_or("<no net>", |net| design.net(net).name.as_str())
}

fn position_in_millimetres(point: Point) -> Value {
    json!({"x": millimetres(point.x), "y": millimetres(point.y)})
}

fn millimetres(nanometres: i32) -> f64 {
    f64::from(nanometres) / 1e6
}

/// `written` in UTC, as RFC 3339 asks.
fn date(written: SystemTime) -> String {
    let seconds = written
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let (days, time_of_day) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil_date(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time_of_day / 3600,
        time_of_day / 60 % 60,
        time_of_day % 60
    )
}

/// The Gregorian year, month and day `days` after 1970-01-01, by Howard
/// Hinnant's `civil_from_days` (public domain), which counts in 400-year eras
/// that start on 1 March.
fn civil_date(days: u64) -> (u64, u64, u64) {
    let since_era_start = days + 719_468;
    let era = since_era_start / 146_097;
    let day_of_era = since_era_start % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    (era * 400 + year_of_era + u64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};
    use traxide_contracts::testing::{DesignBuilder, at, mm};
    use traxide_contracts::{CopperItem, LayerId, Track, Violation};

    fn instant(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn a_clearance_violation_and_an_unconnected_pair_read_as_kicad_writes_them() {
        let design = DesignBuilder::new(30.0, 20.0)
            .smd("R1-1", Some("GND"), at(5.0, 10.0), 1.0, 0)
            .tht("J1-1", Some("GND"), at(25.0, 10.0), 1.6)
            .smd("R2-1", None, at(10.0, 4.5), 1.0, 0)
            .build();
        let routes = Routes {
            tracks: vec![Track {
                net: design.net_named("GND"),
                layer: LayerId::from_index(0),
                width: mm(0.25),
                path: vec![at(5.0, 10.0), at(10.0, 5.0), at(13.0, 5.0)],
            }],
            vias: vec![],
        };
        let verdict = Verdict {
            violations: vec![
                Violation::Clearance {
                    layer: LayerId::from_index(0),
                    a: CopperItem::Pad(design.pad_named("R2-1")),
                    b: CopperItem::Track(0),
                    required: mm(0.2),
                },
                Violation::Unconnected {
                    net: design.net_named("GND"),
                    pads: vec![design.pad_named("J1-1")],
                },
            ],
        };

        let report = report(
            "board.routed.kicad_pcb",
            instant(1_792_527_023),
            &design,
            &routes,
            &verdict,
        );

        assert_eq!(report["$schema"], "https://schemas.kicad.org/drc.v1.json");
        assert_eq!(report["source"], "board.routed.kicad_pcb");
        assert_eq!(report["date"], "2026-10-20T20:10:23Z");
        assert_eq!(
            report["kicad_version"],
            format!("traxide {}", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(report["coordinate_units"], "mm");
        assert_eq!(report["schematic_parity"], json!([]));
        assert_eq!(
            report["violations"],
            json!([{
                "type": "clearance",
                "description": "Clearance violation (clearance 0.2000 mm)",
                "severity": "error",
                "items": [
                    {"description": "Pad R2-1 [<no net>] on L1", "pos": {"x": 10.0, "y": 4.5}},
                    {
                        "description": "Track [GND] on L1, length 10.0711 mm",
                        "pos": {"x": 5.0, "y": 10.0}
                    },
                ],
            }])
        );
        assert_eq!(
            report["unconnected_items"],
            json!([{
                "type": "unconnected_items",
                "description": "Missing connection between items",
                "severity": "error",
                "items": [
                    {"description": "Pad R1-1 [GND]", "pos": {"x": 5.0, "y": 10.0}},
                    {"description": "Pad J1-1 [GND]", "pos": {"x": 25.0, "y": 10.0}},
                ],
            }])
        );
    }

    #[test]
    fn each_pad_apart_from_its_net_is_a_missing_connection_to_the_nets_first_pad() {
        let design = DesignBuilder::new(30.0, 20.0)
            .smd("A-1", Some("N"), at(1.0, 1.0), 1.0, 0)
            .smd("B-1", Some("N"), at(2.0, 1.0), 1.0, 0)
            .smd("C-1", Some("N"), at(3.0, 1.0), 1.0, 0)
            .build();
        let verdict = Verdict {
            violations: vec![Violation::Unconnected {
                net: design.net_named("N"),
                pads: vec![design.pad_named("B-1"), design.pad_named("C-1")],
            }],
        };

        let report = report("b.dsn", instant(0), &design, &Routes::default(), &verdict);

        let pairs: Vec<Vec<&str>> = report["unconnected_items"]
            .as_array()
            .expect("a list")
            .iter()
            .map(|entry| {
                entry["items"]
                    .as_array()
                    .expect("a list")
                    .iter()
                    .map(|item| item["description"].as_str().expect("text"))
                    .collect()
            })
            .collect();
        assert_eq!(
            pairs,
            [
                ["Pad A-1 [N]", "Pad B-1 [N]"],
                ["Pad A-1 [N]", "Pad C-1 [N]"]
            ]
        );
        assert_eq!(report["violations"], json!([]));
    }

    #[test]
    fn edge_and_keepout_violations_take_kicads_names_and_vias_name_their_layers() {
        let design = DesignBuilder::new(30.0, 20.0)
            .layers(4)
            .smd("A-1", Some("N"), at(1.0, 1.0), 1.0, 0)
            .build();
        let routes = Routes {
            tracks: vec![],
            vias: vec![traxide_contracts::Via {
                net: design.net_named("N"),
                kind: traxide_contracts::ViaKindId(0),
                position: at(0.1, 7.25),
            }],
        };
        let verdict = Verdict {
            violations: vec![
                Violation::Edge {
                    layer: LayerId::from_index(3),
                    item: CopperItem::Via(0),
                },
                Violation::Keepout {
                    layer: LayerId::from_index(1),
                    item: CopperItem::Via(0),
                },
            ],
        };

        let report = report("b.dsn", instant(0), &design, &routes, &verdict);

        let via = json!({"description": "Via [N] on L1 - L4", "pos": {"x": 0.1, "y": 7.25}});
        assert_eq!(
            report["violations"],
            json!([
                {
                    "type": "copper_edge_clearance",
                    "description": "Board edge clearance violation",
                    "severity": "error",
                    "items": [via],
                },
                {
                    "type": "items_not_allowed",
                    "description": "Items not allowed",
                    "severity": "error",
                    "items": [via],
                },
            ])
        );
    }

    #[test]
    fn holes_too_close_are_kicads_hole_to_hole_warning() {
        let design = DesignBuilder::new(30.0, 20.0)
            .smd("A-1", Some("N"), at(1.0, 1.0), 1.0, 0)
            .build();
        let via = |x| traxide_contracts::Via {
            net: design.net_named("N"),
            kind: traxide_contracts::ViaKindId(0),
            position: at(x, 5.0),
        };
        let routes = Routes {
            tracks: vec![],
            vias: vec![via(5.0), via(5.5)],
        };
        let verdict = Verdict {
            violations: vec![Violation::HoleToHole {
                a: CopperItem::Via(0),
                b: CopperItem::Via(1),
                required: 250_000,
            }],
        };

        let report = report("b.dsn", instant(0), &design, &routes, &verdict);

        let via = |x| json!({"description": "Via [N] on L1 - L2", "pos": {"x": x, "y": 5.0}});
        assert_eq!(
            report["violations"],
            json!([{
                "type": "hole_to_hole",
                "description": "Drilled hole too close to other hole (min 0.2500 mm)",
                "severity": "warning",
                "items": [via(5.0), via(5.5)],
            }])
        );
    }

    #[test]
    fn dates_are_utc_in_rfc_3339() {
        assert_eq!(date(instant(0)), "1970-01-01T00:00:00Z");
        assert_eq!(date(instant(951_782_400)), "2000-02-29T00:00:00Z");
        assert_eq!(date(instant(1_792_527_023)), "2026-10-20T20:10:23Z");
    }
}
