//! Planning Engine: how the work is planned (volatility V5 of
//! docs/Architecture/Volatilities.md). For now: a minimum spanning tree per net,
//! connections from hemmed-in pads first, then shortest first; vias that join
//! pads to their net's planes; and stubs that escape the pins of fine-pitch
//! parts before anything is routed.

mod escape;
mod existing;
mod planes;

pub use escape::{Escaped, escape};
pub use planes::{PlaneJoins, join_planes};

use traxide_contracts::geometry::Point;
use traxide_contracts::{Connection, Design, NetId, Pad, PadId, Plan};

/// Ties are broken by names, never by the order the board lists things in:
/// KiCad exports one board's parts in a different order each time, and the
/// same board should always get the same routes.
pub fn plan(design: &Design) -> Plan {
    let pad_name = |pad: PadId| design.pad(pad).name.as_str();
    let joined = existing::joined_pads(design);
    let mut connections: Vec<Connection> = design
        .nets
        .iter()
        .enumerate()
        .flat_map(|(index, net)| {
            let mut pads = net.pads.clone();
            pads.sort_by_key(|&pad| pad_name(pad));
            spanning_tree(design, NetId::from_index(index), &pads, &joined)
        })
        .collect();
    // The most constrained connections claim their space first. A pad hemmed in
    // by its neighbours, such as a pin of a fine-pitch chip, has few ways out,
    // which later copper easily takes: routed last, its connections spent most
    // of the keyboard's time ripping each other up. Among the rest, short
    // connections have the least room to go elsewhere, and are the cheapest.
    let hemmed_in = hemmed_in_pads(design);
    let is_free = |c: &Connection| !(hemmed_in[c.from.index()] || hemmed_in[c.to.index()]);
    connections.sort_by_cached_key(|c| {
        (
            is_free(c),
            manhattan(design.pad(c.from).position, design.pad(c.to).position),
            design.net(c.net).name.as_str(),
            pad_name(c.from),
            pad_name(c.to),
        )
    });
    Plan { connections }
}

/// For each pad, whether copper of another net comes so close that no track of
/// its net fits between them.
fn hemmed_in_pads(design: &Design) -> Vec<bool> {
    let room_for_a_track = |pad: &Pad| {
        let class = pad
            .net
            .map_or(design.rules.default_class, |net| design.net(net).class);
        let class = &design.rules.classes[class.index()];
        class.track_width + 2 * class.clearance
    };
    let mut hemmed_in = vec![false; design.pads.len()];
    for (a, pad) in design.pads.iter().enumerate() {
        let room = room_for_a_track(pad);
        hemmed_in[a] = design.pads.iter().any(|other| {
            other.net != pad.net
                && pad.copper.iter().any(|mine| {
                    other.copper.iter().any(|theirs| {
                        mine.layer == theirs.layer
                            && mine
                                .shape
                                .bounds()
                                .inflated(room)
                                .intersects(theirs.shape.bounds())
                            && mine.shape.is_closer_than(&theirs.shape, room)
                    })
                })
        });
    }
    hemmed_in
}

/// Prim's algorithm on Manhattan distances, which approximate rectilinear wiring
/// better than Euclidean ones. Ties go to the pad listed first. Pads the board's
/// own copper joins are no distance apart, and need no connection.
fn spanning_tree(design: &Design, net: NetId, pads: &[PadId], joined: &[usize]) -> Vec<Connection> {
    let group = |i: usize| joined[pads[i].index()];
    let distance = |a: usize, b: usize| {
        if group(a) == group(b) {
            0
        } else {
            manhattan(design.pad(pads[a]).position, design.pad(pads[b]).position)
        }
    };
    let mut in_tree = vec![false; pads.len()];
    let mut nearest: Vec<(i64, usize)> = vec![(i64::MAX, 0); pads.len()];
    let mut connections = Vec::new();
    let mut current = 0;
    for _ in 1..pads.len() {
        in_tree[current] = true;
        for other in (0..pads.len()).filter(|&o| !in_tree[o]) {
            let distance = distance(current, other);
            if distance < nearest[other].0 {
                nearest[other] = (distance, current);
            }
        }
        let Some(next) = (0..pads.len())
            .filter(|&o| !in_tree[o])
            .min_by_key(|&o| (nearest[o].0, o))
        else {
            break;
        };
        if group(nearest[next].1) != group(next) {
            connections.push(Connection {
                net,
                from: pads[nearest[next].1],
                to: pads[next],
            });
        }
        current = next;
    }
    connections
}

fn manhattan(a: Point, b: Point) -> i64 {
    i64::from(a.x.abs_diff(b.x)) + i64::from(a.y.abs_diff(b.y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use traxide_contracts::testing::{DesignBuilder, at};
    use traxide_contracts::{LayerId, Track, Via, ViaKindId};

    fn pads_of(design: &Design, plan: &Plan) -> Vec<(String, String)> {
        let name = |p: PadId| design.pad(p).name.clone();
        plan.connections
            .iter()
            .map(|c| (name(c.from), name(c.to)))
            .collect()
    }

    fn pairs(names: &[(&str, &str)]) -> Vec<(String, String)> {
        names
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    #[test]
    fn a_net_of_two_pads_is_one_connection() {
        let design = DesignBuilder::new(30.0, 10.0)
            .smd("A1", Some("A"), at(1.0, 1.0), 0.5, 0)
            .smd("A2", Some("A"), at(9.0, 1.0), 0.5, 0)
            .build();
        assert_eq!(pads_of(&design, &plan(&design)), pairs(&[("A1", "A2")]));
    }

    #[test]
    fn pads_in_a_row_are_chained_rather_than_joined_to_the_first() {
        let design = DesignBuilder::new(30.0, 10.0)
            .smd("A1", Some("A"), at(1.0, 1.0), 0.5, 0)
            .smd("A3", Some("A"), at(21.0, 1.0), 0.5, 0)
            .smd("A2", Some("A"), at(10.0, 1.0), 0.5, 0)
            .build();
        assert_eq!(
            pads_of(&design, &plan(&design)),
            pairs(&[("A1", "A2"), ("A2", "A3")])
        );
    }

    #[test]
    fn single_pads_need_no_connection() {
        let design = DesignBuilder::new(30.0, 10.0)
            .smd("A1", Some("A"), at(1.0, 1.0), 0.5, 0)
            .build();
        assert!(plan(&design).connections.is_empty());
    }

    #[test]
    fn connections_from_hemmed_in_pads_come_before_shorter_ones() {
        // P1 sits between two pads of other nets, 0.4 mm apart like the pins of
        // a fine-pitch chip: no track fits between them.
        let design = DesignBuilder::new(40.0, 20.0)
            .track_width(0.2)
            .clearance(0.15)
            .smd("X1", Some("X"), at(5.0, 4.6), 0.2, 0)
            .smd("P1", Some("P"), at(5.0, 5.0), 0.2, 0)
            .smd("Y1", Some("Y"), at(5.0, 5.4), 0.2, 0)
            .smd("P2", Some("P"), at(35.0, 5.0), 0.5, 0)
            .smd("S1", Some("S"), at(10.0, 15.0), 0.5, 0)
            .smd("S2", Some("S"), at(12.0, 15.0), 0.5, 0)
            .build();
        assert_eq!(
            pads_of(&design, &plan(&design)),
            pairs(&[("P1", "P2"), ("S1", "S2")])
        );
    }

    #[test]
    fn pads_with_room_for_a_track_between_them_are_not_hemmed_in() {
        let design = DesignBuilder::new(40.0, 20.0)
            .track_width(0.2)
            .clearance(0.15)
            .smd("X1", Some("X"), at(5.0, 4.0), 0.2, 0)
            .smd("P1", Some("P"), at(5.0, 5.0), 0.2, 0)
            .smd("P2", Some("P"), at(35.0, 5.0), 0.5, 0)
            .smd("S1", Some("S"), at(10.0, 15.0), 0.5, 0)
            .smd("S2", Some("S"), at(12.0, 15.0), 0.5, 0)
            .build();
        assert_eq!(
            pads_of(&design, &plan(&design)),
            pairs(&[("S1", "S2"), ("P1", "P2")])
        );
    }

    #[test]
    fn the_plan_does_not_depend_on_the_order_the_board_lists_things_in() {
        // KiCad exports the same board's parts in a different order each time.
        // A3 is as far from A1 as from A2, and A1–A2 as long as B1–B2: ties
        // that the order of listing must not break.
        let one = DesignBuilder::new(30.0, 10.0)
            .smd("A1", Some("A"), at(1.0, 1.0), 0.5, 0)
            .smd("A2", Some("A"), at(5.0, 1.0), 0.5, 0)
            .smd("A3", Some("A"), at(3.0, 4.0), 0.5, 0)
            .smd("B1", Some("B"), at(11.0, 1.0), 0.5, 0)
            .smd("B2", Some("B"), at(15.0, 1.0), 0.5, 0)
            .build();
        let other = DesignBuilder::new(30.0, 10.0)
            .smd("B2", Some("B"), at(15.0, 1.0), 0.5, 0)
            .smd("A3", Some("A"), at(3.0, 4.0), 0.5, 0)
            .smd("B1", Some("B"), at(11.0, 1.0), 0.5, 0)
            .smd("A2", Some("A"), at(5.0, 1.0), 0.5, 0)
            .smd("A1", Some("A"), at(1.0, 1.0), 0.5, 0)
            .build();
        assert_eq!(pads_of(&one, &plan(&one)), pads_of(&other, &plan(&other)));
    }

    #[test]
    fn pads_the_boards_own_copper_joins_need_no_connection() {
        let mut design = DesignBuilder::new(30.0, 10.0)
            .smd("A1", Some("A"), at(1.0, 1.0), 0.5, 0)
            .smd("A2", Some("A"), at(9.0, 1.0), 0.5, 0)
            .smd("A3", Some("A"), at(9.0, 6.0), 0.5, 0)
            .build();
        design.existing.tracks.push(Track {
            net: design.net_named("A"),
            layer: LayerId(0),
            width: 200_000,
            path: vec![at(1.0, 1.0), at(9.0, 1.0)],
        });
        assert_eq!(pads_of(&design, &plan(&design)), pairs(&[("A2", "A3")]));
    }

    #[test]
    fn a_via_joins_the_boards_own_copper_across_layers() {
        let mut design = DesignBuilder::new(30.0, 10.0)
            .smd("A1", Some("A"), at(1.0, 1.0), 0.5, 0)
            .smd("A2", Some("A"), at(9.0, 1.0), 0.5, 1)
            .build();
        let net = design.net_named("A");
        let track = |layer, path| Track {
            net,
            layer: LayerId(layer),
            width: 200_000,
            path,
        };
        design
            .existing
            .tracks
            .push(track(0, vec![at(1.0, 1.0), at(5.0, 1.0)]));
        design
            .existing
            .tracks
            .push(track(1, vec![at(5.0, 1.0), at(9.0, 1.0)]));
        design.existing.vias.push(Via {
            net,
            kind: ViaKindId(0),
            position: at(5.0, 1.0),
        });
        assert!(plan(&design).connections.is_empty());
    }

    #[test]
    fn copper_of_another_layer_joins_nothing_without_a_via() {
        let mut design = DesignBuilder::new(30.0, 10.0)
            .smd("A1", Some("A"), at(1.0, 1.0), 0.5, 0)
            .smd("A2", Some("A"), at(9.0, 1.0), 0.5, 0)
            .build();
        design.existing.tracks.push(Track {
            net: design.net_named("A"),
            layer: LayerId(1),
            width: 200_000,
            path: vec![at(1.0, 1.0), at(9.0, 1.0)],
        });
        assert_eq!(pads_of(&design, &plan(&design)), pairs(&[("A1", "A2")]));
    }

    #[test]
    fn through_hole_pads_on_a_plane_of_their_net_need_no_connection() {
        let design = DesignBuilder::new(30.0, 10.0)
            .layers(3)
            .plane_layer(1)
            .plane("GND", 1, at(0.0, 0.0), at(20.0, 10.0))
            .tht("J1", Some("GND"), at(2.0, 2.0), 1.6)
            .tht("J2", Some("GND"), at(18.0, 8.0), 1.6)
            .tht("J3", Some("GND"), at(25.0, 8.0), 1.6)
            .build();
        assert_eq!(pads_of(&design, &plan(&design)), pairs(&[("J2", "J3")]));
    }

    #[test]
    fn a_plane_joins_its_pads_where_a_plane_of_another_net_of_lower_priority_overlaps_it() {
        let design = DesignBuilder::new(30.0, 10.0)
            .layers(3)
            .plane_layer(1)
            .plane("VCC", 1, at(0.0, 0.0), at(30.0, 10.0))
            .plane_of_priority("GND", 1, at(0.0, 0.0), at(20.0, 10.0), 2)
            .tht("J1", Some("GND"), at(2.0, 2.0), 1.6)
            .tht("J2", Some("GND"), at(18.0, 8.0), 1.6)
            .tht("J3", Some("GND"), at(25.0, 8.0), 1.6)
            .tht("V1", Some("VCC"), at(25.0, 2.0), 1.6)
            .build();
        assert_eq!(pads_of(&design, &plan(&design)), pairs(&[("J2", "J3")]));
    }

    #[test]
    fn a_via_of_the_board_on_a_plane_joins_the_copper_it_carries() {
        let mut design = DesignBuilder::new(30.0, 10.0)
            .layers(3)
            .plane_layer(1)
            .plane("GND", 1, at(0.0, 0.0), at(30.0, 10.0))
            .smd("G1", Some("GND"), at(2.0, 2.0), 0.5, 0)
            .tht("J1", Some("GND"), at(25.0, 8.0), 1.6)
            .build();
        let net = design.net_named("GND");
        design.existing.tracks.push(Track {
            net,
            layer: LayerId(0),
            width: 200_000,
            path: vec![at(2.0, 2.0), at(3.0, 2.0)],
        });
        design.existing.vias.push(Via {
            net,
            kind: ViaKindId(0),
            position: at(3.0, 2.0),
        });
        assert!(plan(&design).connections.is_empty());
    }

    #[test]
    fn the_shortest_connections_come_first() {
        let design = DesignBuilder::new(30.0, 10.0)
            .smd("L1", Some("Long"), at(1.0, 1.0), 0.5, 0)
            .smd("L2", Some("Long"), at(25.0, 1.0), 0.5, 0)
            .smd("S1", Some("Short"), at(1.0, 5.0), 0.5, 0)
            .smd("S2", Some("Short"), at(3.0, 5.0), 0.5, 0)
            .build();
        assert_eq!(
            pads_of(&design, &plan(&design)),
            pairs(&[("S1", "S2"), ("L1", "L2")])
        );
    }

    #[test]
    fn equal_distances_are_decided_by_order_not_by_chance() {
        let design = DesignBuilder::new(30.0, 30.0)
            .smd("C", Some("A"), at(10.0, 10.0), 0.5, 0)
            .smd("E", Some("A"), at(15.0, 10.0), 0.5, 0)
            .smd("W", Some("A"), at(5.0, 10.0), 0.5, 0)
            .build();
        assert_eq!(
            pads_of(&design, &plan(&design)),
            pairs(&[("C", "E"), ("C", "W")])
        );
    }
}
