//! The connections of differential pairs that run side by side, to be routed
//! as one.

use traxide_contracts::{Connection, ConstraintSet, CoupledConnections, Design, PadId, Plan};

/// How far apart, in pair pitches (a track's width and the gap), the pads at
/// each end of two connections of a pair may lie for the connections to be
/// coupled: as far as the pins of a SOT-23's side, 1.9 mm apart for USB's
/// 0.45 mm pitch, and no farther, as pads farther apart are other ends.
const BESIDE_IN_PAIR_PITCHES: i64 = 6;

/// The connections of `plan` of each differential pair of `rules` that start
/// beside each other and end beside each other, each taken once, in the
/// plan's order. Not those between pins of one part, as the two rows of a
/// USB-C receptacle are: they join beside it, each its own way.
pub fn coupled(design: &Design, rules: &ConstraintSet, plan: &Plan) -> Vec<CoupledConnections> {
    let mut taken = vec![false; plan.connections.len()];
    let mut coupled = Vec::new();
    for pair in rules.pairs() {
        let beside = |a: PadId, b: PadId| {
            design.pad(a).position.distance(design.pad(b).position)
                <= BESIDE_IN_PAIR_PITCHES * i64::from(pair.width + pair.gap)
        };
        let within_a_part = |c: &Connection| design.pad(c.from).part() == design.pad(c.to).part();
        for (p, positive) in plan.connections.iter().enumerate() {
            if positive.net != pair.positive || taken[p] || within_a_part(positive) {
                continue;
            }
            let partner = plan
                .connections
                .iter()
                .enumerate()
                .find_map(|(n, negative)| {
                    if negative.net != pair.negative || taken[n] {
                        return None;
                    }
                    let reversed = Connection {
                        from: negative.to,
                        to: negative.from,
                        ..*negative
                    };
                    [*negative, reversed]
                        .into_iter()
                        .find(|c| beside(positive.from, c.from) && beside(positive.to, c.to))
                        .map(|c| (n, c))
                });
            if let Some((n, negative)) = partner {
                taken[p] = true;
                taken[n] = true;
                coupled.push(CoupledConnections {
                    pair: *pair,
                    connections: [*positive, negative],
                });
            }
        }
    }
    coupled
}

#[cfg(test)]
mod tests {
    use super::*;
    use traxide_contracts::testing::{DesignBuilder, at};
    use traxide_rules::Purpose;

    fn coupled_in(design: &Design) -> Vec<CoupledConnections> {
        let rules = traxide_rules::compile(design, Purpose::Verification);
        coupled(design, &rules, &crate::plan(design))
    }

    fn connection(design: &Design, from: &str, to: &str) -> Connection {
        let from = design.pad_named(from);
        Connection {
            net: design.pad(from).net.expect("pads of a net"),
            from,
            to: design.pad_named(to),
        }
    }

    #[test]
    fn connections_of_a_pair_that_start_and_end_side_by_side_are_coupled() {
        let design = DesignBuilder::new(20.0, 10.0)
            .differential_pairs(0.2, 0.25)
            .smd("P1", Some("D_P"), at(2.0, 4.5), 0.5, 0)
            .smd("P2", Some("D_P"), at(18.0, 4.5), 0.5, 0)
            // Listed the other way, as the names order them.
            .smd("NB", Some("D_N"), at(2.0, 5.5), 0.5, 0)
            .smd("NA", Some("D_N"), at(18.0, 5.5), 0.5, 0)
            .build();
        let coupled = coupled_in(&design);
        assert_eq!(coupled.len(), 1);
        assert_eq!(
            coupled[0].connections,
            [
                connection(&design, "P1", "P2"),
                connection(&design, "NB", "NA")
            ]
        );
    }

    #[test]
    fn connections_of_a_pair_between_pins_of_one_part_are_not_coupled() {
        let design = DesignBuilder::new(20.0, 10.0)
            .differential_pairs(0.2, 0.25)
            .smd("J1-A6", Some("D_P"), at(9.75, 4.0), 0.3, 0)
            .smd("J1-A7", Some("D_N"), at(10.25, 4.0), 0.3, 0)
            .smd("J1-B6", Some("D_P"), at(10.75, 4.0), 0.3, 0)
            .smd("J1-B7", Some("D_N"), at(9.25, 4.0), 0.3, 0)
            .build();
        assert_eq!(coupled_in(&design), []);
    }

    #[test]
    fn connections_of_a_pair_that_end_apart_are_not_coupled() {
        let design = DesignBuilder::new(20.0, 10.0)
            .differential_pairs(0.2, 0.25)
            .smd("P1", Some("D_P"), at(2.0, 4.5), 0.5, 0)
            .smd("P2", Some("D_P"), at(18.0, 4.5), 0.5, 0)
            .smd("N1", Some("D_N"), at(2.0, 5.5), 0.5, 0)
            .smd("N2", Some("D_N"), at(10.0, 9.0), 0.5, 0)
            .build();
        assert_eq!(coupled_in(&design), []);
    }
}
