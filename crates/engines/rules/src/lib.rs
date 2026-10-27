//! Rules Engine: what the design rules mean (volatility V4 of
//! docs/Architecture/Volatilities.md), compiled once into a [`ConstraintSet`].

mod custom;

use custom::{Groups, Rules};
use traxide_contracts::{
    Constraint, ConstraintSet, Design, Limits, NetClassId, ViaKind, ViaKindId, ViaLimits,
};

/// What the compiled rules are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// For routing on a grid of `grid_pitch`. Stored routes are rounded to the
    /// design's resolution, which can move copper towards its neighbours by up
    /// to a step: clearances grow by one, unless the grid is made of whole steps,
    /// as on boards from KiCad. Then nothing moves, and copper may fit exactly
    /// where the rules allow, as between the pins of a fine-pitch chip.
    Routing { grid_pitch: i32 },
    /// The rules exactly as the design states them, as KiCad's DRC applies them.
    Verification,
}

/// The rules of `design`: its classes' values, overridden by its custom rules.
/// A custom rule that may or may not apply, because its condition cannot be
/// told, only ever keeps copper further apart.
pub fn compile(design: &Design, purpose: Purpose) -> ConstraintSet {
    let margin = match purpose {
        Purpose::Routing { grid_pitch }
            if design.resolution > 0 && grid_pitch % design.resolution != 0 =>
        {
            design.resolution
        }
        Purpose::Routing { .. } | Purpose::Verification => 0,
    };
    let Groups {
        groups,
        of_net,
        of_no_net,
    } = Groups::of(design);
    let rules = Rules::new(design, &groups);
    let ids: Vec<NetClassId> = (0..groups.len()).map(NetClassId::from_index).collect();
    let class = |group: NetClassId| &design.rules.classes[groups[group.index()].class.index()];
    let via_kinds: Vec<Vec<ViaKindId>> = ids
        .iter()
        .map(|&group| {
            let diameter = rules.of(Constraint::ViaDiameter, group);
            let drill = rules.of(Constraint::HoleSize, group);
            class(group)
                .via_kinds
                .iter()
                .copied()
                .filter(|&kind| {
                    let kind = design.via_kind(kind);
                    diameter.allows(Some(diameter_of(kind))) && drill.allows(kind.drill)
                })
                .collect()
        })
        .collect();
    // Other copper keeps its clearance from a via's copper, so from its hole
    // also by the via's ring.
    let thinnest_ring = |group: NetClassId| {
        via_kinds[group.index()]
            .iter()
            .map(|&kind| design.via_kind(kind))
            .filter_map(|kind| Some((diameter_of(kind) - kind.drill?) / 2))
            .min()
    };
    let clearance_between_classes = ids
        .iter()
        .flat_map(|&a| ids.iter().map(move |&b| (a, b)))
        .map(|(a, b)| {
            let copper = rules
                .between(Constraint::Clearance, a, b)
                .minimum(class(a).clearance.max(class(b).clearance));
            let hole = rules
                .between(Constraint::HoleClearance, a, b)
                .minimum(design.rules.hole_clearance);
            [a, b]
                .into_iter()
                .filter_map(|via_net| thinnest_ring(via_net).map(|ring| hole - ring))
                .fold(copper, i32::max)
                + margin
        })
        .collect();
    let edge_clearance_of = |footprint: Option<&str>| {
        ids.iter()
            .map(|&group| {
                rules
                    .at_edge(group, footprint)
                    .minimum(design.rules.edge_clearance)
            })
            .max()
            .unwrap_or(design.rules.edge_clearance)
            + margin
            + design.outline_tolerance
    };
    let edge_clearance = edge_clearance_of(None);
    let cutout_clearances = (0..design.outline.holes().len())
        .map(|hole| {
            let footprint = design.cutout_footprints.get(hole).cloned().flatten();
            edge_clearance_of(footprint.as_deref())
        })
        .collect();
    ConstraintSet::new(
        of_net,
        of_no_net,
        ids.iter()
            .map(|&group| {
                rules
                    .of(Constraint::TrackWidth, group)
                    .value(class(group).track_width)
            })
            .collect(),
        via_kinds,
        clearance_between_classes,
        edge_clearance,
    )
    .with_cutout_clearances(cutout_clearances)
    .with_hole_to_hole(match design.rules.hole_to_hole {
        0 => 0,
        apart => apart + margin,
    })
    .with_track_width_limits(
        ids.iter()
            .map(|&group| {
                limits_to_judge_by(
                    &rules,
                    Constraint::TrackWidth,
                    group,
                    design.rules.min_track_width,
                )
            })
            .collect(),
    )
    .with_via_limits(
        ids.iter()
            .map(|&group| ViaLimits {
                diameter: limits_to_judge_by(
                    &rules,
                    Constraint::ViaDiameter,
                    group,
                    design.rules.min_via_diameter,
                ),
                drill: limits_to_judge_by(
                    &rules,
                    Constraint::HoleSize,
                    group,
                    design.rules.min_via_drill,
                ),
            })
            .collect(),
    )
}

/// The limits `constraint` sets on the copper of `group`: the board's
/// minimum, and the last rule that surely applies to it. A rule that only
/// may apply, as the reader could not tell, limits nothing to judge by.
fn limits_to_judge_by(
    rules: &Rules,
    constraint: Constraint,
    group: NetClassId,
    board_minimum: i32,
) -> Limits {
    let stated = rules
        .of(constraint, group)
        .last
        .copied()
        .unwrap_or_default();
    let board = Some(board_minimum).filter(|&min| min > 0);
    Limits {
        min: stated.min.max(board),
        opt: None,
        max: stated.max,
    }
}

/// The width of a via's largest copper.
fn diameter_of(kind: &ViaKind) -> i32 {
    kind.copper
        .iter()
        .map(|c| {
            let bounds = c.shape.bounds();
            (bounds.max.x - bounds.min.x).max(bounds.max.y - bounds.min.y)
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use traxide_contracts::geometry::{Point, Polygon, Rect, Shape};
    use traxide_contracts::{
        Condition, Constraint, CustomRule, DesignRules, Item, Layer, LayerId, LayerShape, Limits,
        Net, NetClass, NetClassId, NetId, Routes, ViaKind, ViaKindId,
    };

    fn class(name: &str, track_width: i32, clearance: i32, via: u32) -> NetClass {
        NetClass {
            name: name.into(),
            track_width,
            clearance,
            via_kinds: vec![ViaKindId(via)],
        }
    }

    fn via(diameter: i32, drill: i32) -> ViaKind {
        ViaKind {
            name: format!("{diameter}:{drill}"),
            copper: (0..2)
                .map(|layer| LayerShape {
                    layer: LayerId(layer),
                    shape: Shape::circle(Point::default(), diameter),
                })
                .collect(),
            drill: Some(drill),
        }
    }

    fn design(resolution: i32) -> Design {
        Design {
            layers: ["top", "bottom"]
                .map(|name| Layer {
                    name: name.into(),
                    routable: true,
                })
                .into(),
            outline: Polygon::rectangle(Rect {
                min: Point::new(0, 0),
                max: Point::new(1, 1),
            })
            .into(),
            cutout_footprints: Vec::new(),
            locked: Routes::default(),
            nets: vec![
                Net {
                    name: "signal".into(),
                    class: NetClassId(0),
                    pads: vec![],
                },
                Net {
                    name: "power".into(),
                    class: NetClassId(1),
                    pads: vec![],
                },
                Net {
                    name: "power2".into(),
                    class: NetClassId(1),
                    pads: vec![],
                },
            ],
            pads: vec![],
            via_kinds: vec![via(600, 300), via(800, 400)],
            keepouts: vec![],
            planes: vec![],
            existing: Routes::default(),
            rules: DesignRules {
                classes: vec![class("default", 200, 200, 0), class("power", 500, 300, 1)],
                default_class: NetClassId(0),
                edge_clearance: 250,
                hole_clearance: 0,
                hole_to_hole: 0,
                min_track_width: 0,
                min_via_diameter: 0,
                min_via_drill: 0,
                custom: vec![],
            },
            resolution,
            outline_tolerance: 0,
            warnings: Vec::new(),
        }
    }

    const SIGNAL: Option<NetId> = Some(NetId(0));
    const POWER: Option<NetId> = Some(NetId(1));
    const POWER2: Option<NetId> = Some(NetId(2));
    const DEFAULT_CLASS: NetClassId = NetClassId(0);
    const POWER_CLASS: NetClassId = NetClassId(1);

    fn rule(constraint: Constraint, condition: Condition, min: i32) -> CustomRule {
        CustomRule {
            name: "custom".into(),
            condition,
            constraint,
            limits: Limits {
                min: Some(min),
                ..Limits::default()
            },
        }
    }

    fn class_is(item: Item, class: NetClassId) -> Condition {
        Condition::NetClassIn {
            item,
            classes: vec![class],
        }
    }

    fn with_rules(rules: Vec<CustomRule>) -> ConstraintSet {
        let mut design = design(0);
        design.rules.custom = rules;
        compile(&design, Purpose::Verification)
    }

    #[test]
    fn a_rule_between_two_classes_sets_their_clearance() {
        let rules = with_rules(vec![rule(
            Constraint::Clearance,
            Condition::All(vec![
                class_is(Item::A, DEFAULT_CLASS),
                class_is(Item::B, POWER_CLASS),
            ]),
            1_000,
        )]);
        assert_eq!(rules.clearance(SIGNAL, POWER), 1_000);
        assert_eq!(rules.clearance(POWER, SIGNAL), 1_000);
        assert_eq!(rules.clearance(POWER, POWER2), 300);
        assert_eq!(rules.clearance(SIGNAL, SIGNAL), 200);
    }

    #[test]
    fn a_rule_about_one_item_applies_whichever_way_round() {
        let rules = with_rules(vec![rule(
            Constraint::Clearance,
            class_is(Item::B, POWER_CLASS),
            1_000,
        )]);
        assert_eq!(rules.clearance(POWER, SIGNAL), 1_000);
        assert_eq!(rules.clearance(SIGNAL, POWER), 1_000);
        assert_eq!(rules.clearance(SIGNAL, SIGNAL), 200);
    }

    #[test]
    fn the_last_rule_that_applies_counts_even_below_the_classes() {
        let rules = with_rules(vec![
            rule(Constraint::Clearance, Condition::Always, 900),
            rule(Constraint::Clearance, Condition::Always, 100),
        ]);
        assert_eq!(rules.clearance(SIGNAL, POWER), 100);
    }

    #[test]
    fn a_rule_naming_a_net_applies_to_that_net_alone() {
        let rules = with_rules(vec![rule(
            Constraint::Clearance,
            Condition::NetIn {
                item: Item::A,
                nets: vec![POWER],
            },
            700,
        )]);
        assert_eq!(rules.clearance(SIGNAL, POWER), 700);
        assert_eq!(rules.clearance(SIGNAL, POWER2), 300);
        assert_eq!(rules.track_width(NetId(2)), 500);
    }

    #[test]
    fn copper_of_no_net_belongs_to_the_default_class_and_has_no_name() {
        let no_net = Condition::NetIn {
            item: Item::A,
            nets: vec![None],
        };
        let rules = with_rules(vec![rule(Constraint::Clearance, no_net, 800)]);
        assert_eq!(rules.clearance(None, POWER), 800);
        assert_eq!(rules.clearance(SIGNAL, POWER), 300);
        let rules = with_rules(vec![rule(
            Constraint::Clearance,
            class_is(Item::A, DEFAULT_CLASS),
            800,
        )]);
        assert_eq!(rules.clearance(None, POWER), 800);
    }

    #[test]
    fn a_rule_that_may_apply_only_ever_widens() {
        let rules = with_rules(vec![
            rule(Constraint::Clearance, Condition::Always, 400),
            rule(Constraint::Clearance, Condition::Unknown, 100),
        ]);
        assert_eq!(rules.clearance(SIGNAL, POWER), 400);
        let rules = with_rules(vec![
            rule(Constraint::Clearance, Condition::Always, 400),
            rule(Constraint::Clearance, Condition::Unknown, 900),
        ]);
        assert_eq!(rules.clearance(SIGNAL, POWER), 900);
        let rules = with_rules(vec![rule(
            Constraint::Clearance,
            Condition::Not(Box::new(Condition::Unknown)),
            100,
        )]);
        assert_eq!(rules.clearance(SIGNAL, POWER), 300);
    }

    #[test]
    fn a_rule_on_some_layers_may_apply_and_one_on_every_layer_does() {
        let on = |layers: Vec<u32>| Condition::OnLayers(layers.into_iter().map(LayerId).collect());
        let some = with_rules(vec![rule(Constraint::Clearance, on(vec![0]), 100)]);
        assert_eq!(some.clearance(SIGNAL, POWER), 300);
        let every = with_rules(vec![rule(Constraint::Clearance, on(vec![0, 1]), 100)]);
        assert_eq!(every.clearance(SIGNAL, POWER), 100);
        let none = with_rules(vec![rule(Constraint::Clearance, on(vec![]), 900)]);
        assert_eq!(none.clearance(SIGNAL, POWER), 300);
    }

    #[test]
    fn a_clearance_rule_without_a_minimum_changes_nothing() {
        let mut maximum = rule(Constraint::Clearance, Condition::Always, 0);
        maximum.limits = Limits {
            max: Some(100),
            ..Limits::default()
        };
        let rules = with_rules(vec![
            rule(Constraint::Clearance, Condition::Always, 800),
            maximum,
        ]);
        assert_eq!(rules.clearance(SIGNAL, POWER), 800);
    }

    #[test]
    fn track_widths_take_the_rules_optimum_within_its_limits() {
        let limits = |min, opt, max| CustomRule {
            limits: Limits { min, opt, max },
            ..rule(Constraint::TrackWidth, class_is(Item::A, POWER_CLASS), 0)
        };
        let optimum = with_rules(vec![limits(None, Some(350), None)]);
        assert_eq!(optimum.track_width(NetId(1)), 350);
        assert_eq!(optimum.track_width(NetId(0)), 200);
        let widened = with_rules(vec![limits(Some(600), None, None)]);
        assert_eq!(widened.track_width(NetId(1)), 600);
        let narrowed = with_rules(vec![limits(None, None, Some(150))]);
        assert_eq!(narrowed.track_width(NetId(1)), 150);
    }

    #[test]
    fn a_net_avoids_vias_a_rule_forbids() {
        let rules = with_rules(vec![rule(
            Constraint::ViaDiameter,
            class_is(Item::A, DEFAULT_CLASS),
            700,
        )]);
        assert!(rules.via_kinds(NetId(0)).is_empty());
        assert_eq!(rules.via_kinds(NetId(1)), [ViaKindId(1)]);
        let rules = with_rules(vec![rule(Constraint::HoleSize, Condition::Unknown, 350)]);
        assert!(rules.via_kinds(NetId(0)).is_empty());
        assert_eq!(rules.via_kinds(NetId(1)), [ViaKindId(1)]);
    }

    #[test]
    fn copper_keeps_the_hole_clearance_from_vias_beyond_their_ring() {
        let rules = with_rules(vec![rule(
            Constraint::HoleClearance,
            Condition::Always,
            500,
        )]);
        assert_eq!(rules.clearance(SIGNAL, SIGNAL), 350);
        assert_eq!(rules.clearance(POWER, POWER2), 300);
    }

    #[test]
    fn the_edge_keeps_the_largest_clearance_any_rule_asks_and_rules_may_lower_it() {
        let wider = with_rules(vec![rule(
            Constraint::EdgeClearance,
            class_is(Item::A, POWER_CLASS),
            900,
        )]);
        assert_eq!(wider.edge_clearance(), 900);
        let lower = with_rules(vec![rule(
            Constraint::EdgeClearance,
            Condition::Always,
            100,
        )]);
        assert_eq!(lower.edge_clearance(), 100);
    }

    #[test]
    fn nets_of_two_classes_keep_the_larger_clearance() {
        let rules = compile(&design(0), Purpose::Routing { grid_pitch: 100 });
        assert_eq!(rules.clearance(SIGNAL, POWER), 300);
        assert_eq!(rules.clearance(POWER, SIGNAL), 300);
        assert_eq!(rules.clearance(SIGNAL, SIGNAL), 200);
    }

    #[test]
    fn copper_of_no_net_follows_the_default_class() {
        assert_eq!(
            compile(&design(0), Purpose::Routing { grid_pitch: 100 }).clearance(None, SIGNAL),
            200
        );
    }

    #[test]
    fn clearances_include_the_rounding_of_stored_routes_off_the_resolution() {
        let rules = compile(&design(100), Purpose::Routing { grid_pitch: 150 });
        assert_eq!(rules.clearance(SIGNAL, POWER), 400);
        assert_eq!(rules.edge_clearance(), 350);
    }

    #[test]
    fn routes_on_a_grid_of_whole_resolution_steps_may_fit_exactly() {
        let rules = compile(&design(100), Purpose::Routing { grid_pitch: 1_000 });
        assert_eq!(rules.clearance(SIGNAL, POWER), 300);
        assert_eq!(rules.edge_clearance(), 250);
    }

    #[test]
    fn verification_applies_the_rules_as_stated() {
        let rules = compile(&design(100), Purpose::Verification);
        assert_eq!(rules.clearance(SIGNAL, POWER), 300);
        assert_eq!(rules.edge_clearance(), 250);
    }

    #[test]
    fn widths_and_vias_follow_the_class_of_the_net() {
        let rules = compile(&design(100), Purpose::Routing { grid_pitch: 100 });
        assert_eq!(rules.track_width(NetId(1)), 500);
        assert_eq!(rules.via_kinds(NetId(1)), [ViaKindId(1)]);
    }

    #[test]
    fn the_largest_clearance_bounds_every_query() {
        assert_eq!(
            compile(&design(100), Purpose::Routing { grid_pitch: 150 }).largest_clearance(),
            400
        );
    }

    #[test]
    fn the_board_edge_does_not_widen_the_clearance_between_copper() {
        let mut wide_edge = design(1);
        wide_edge.rules.edge_clearance = 900;
        let rules = compile(&wide_edge, Purpose::Verification);
        assert_eq!(
            (rules.largest_clearance(), rules.edge_clearance()),
            (300, 900)
        );
    }
}
