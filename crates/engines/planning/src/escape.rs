//! Fan-out: the pins of fine-pitch parts escape first, each by a short stub
//! straight away from its part to where a track may turn any way, before any
//! connection is routed. Between such pins no track fits, so their only ways
//! out lead along their axes, which the copper of other connections, routed
//! earlier, easily takes; a stub keeps the way out for its pin's net.

use traxide_contracts::geometry::{Point, Segment, Shape};
use traxide_contracts::{
    Connection, ConnectionId, ConstraintSet, Design, LayerShape, LayoutAccess, Pad, PadId, Plan,
    Routes, Track,
};

/// Parts with fewer pins are passives and small transistors, whose pads are
/// close together but easy to reach from the side.
const FINE_PITCH_PART_PINS: usize = 8;

/// How long a stub may grow, in track widths, before its pin is left without
/// an escape.
const LONGEST_STUB_IN_TRACK_WIDTHS: i32 = 12;

/// The design and plan after fan-out: each escaped pin has a stub, and the
/// connections that leave its part start at the stub's end instead.
#[derive(Debug, Clone)]
pub struct Escaped {
    /// The design with one pad more per escape, of the pin's net, whose copper
    /// is the stub's, so that no other net takes it.
    pub design: Design,
    pub plan: Plan,
    escapes: Vec<Escape>,
    pads_before: usize,
}

#[derive(Debug, Clone)]
struct Escape {
    pin: PadId,
    stub: Track,
}

impl Escaped {
    /// The pad of the design before fan-out that `pad` stands for: its pin,
    /// for the pad at the end of an escape.
    pub fn original(&self, pad: PadId) -> PadId {
        pad.index()
            .checked_sub(self.pads_before)
            .map_or(pad, |escape| self.escapes[escape].pin)
    }

    /// `connection` between the pads it stands for, the pins of its escapes.
    pub fn at_pins(&self, connection: Connection) -> Connection {
        Connection {
            from: self.original(connection.from),
            to: self.original(connection.to),
            ..connection
        }
    }

    /// The stubs that a routed connection starts from: a stub all of whose
    /// connections are among `unrouted` would only dangle, and one that lies
    /// on a lane of the board's wiring would end dangling in its middle, as
    /// the lane leads the pin out already. Each ends where the route of
    /// `routes` farthest from its pin joins it.
    pub fn copper_used(&self, unrouted: &[Connection], routes: &Routes) -> Routes {
        let tracks = self
            .escapes
            .iter()
            .enumerate()
            .filter(|&(index, _)| {
                let end = PadId::from_index(self.pads_before + index);
                self.plan
                    .connections
                    .iter()
                    .any(|c| (c.from == end || c.to == end) && !unrouted.contains(c))
            })
            .map(|(_, escape)| &escape.stub)
            .filter(|stub| !self.is_wired_along(stub))
            .filter_map(|stub| trimmed(stub, routes))
            .collect();
        Routes {
            tracks,
            vias: Vec::new(),
        }
    }

    /// Whether a track of the board's wiring of the stub's net covers it.
    fn is_wired_along(&self, stub: &Track) -> bool {
        self.design.existing.tracks.iter().any(|track| {
            track.net == stub.net
                && track.layer == stub.layer
                && track.width >= stub.width
                && track.path.windows(2).any(|pair| {
                    let shape = Shape::track(Segment::new(pair[0], pair[1]), track.width);
                    stub.path.iter().all(|&p| shape.nearest_point(p) == p)
                })
        })
    }

    /// Adds the pad at the end of `stub` that connections of `pin` leave from.
    fn add_end(&mut self, pin: &Pad, stub: &Track) -> PadId {
        let net = pin.net.expect("only pins of a net escape");
        let end = PadId::from_index(self.design.pads.len());
        self.design.pads.push(Pad {
            name: format!("{}~escape", pin.name),
            net: Some(net),
            position: stub.path[1],
            copper: vec![LayerShape {
                layer: stub.layer,
                shape: Shape::track(Segment::new(stub.path[0], stub.path[1]), stub.width),
            }],
        });
        self.design.nets[net.index()].pads.push(end);
        end
    }
}

/// `stub` up to where the track of `routes` farthest from its pin ends on it,
/// as the pad at its end is all of its copper and a route may join it
/// anywhere: beyond that the stub would lead nowhere. Unchanged where no
/// track ends on it; none where they all end at the pin.
fn trimmed(stub: &Track, routes: &Routes) -> Option<Track> {
    let [pin, end] = [stub.path[0], stub.path[1]];
    let along = Segment::new(pin, end);
    let (dx, dy) = (f64::from(end.x - pin.x), f64::from(end.y - pin.y));
    let length_squared = dx * dx + dy * dy;
    let farthest = routes
        .tracks
        .iter()
        .filter(|t| t.net == stub.net && t.layer == stub.layer && t.path.len() >= 2)
        .flat_map(|t| [(t.path[0], t.width), (t.path[t.path.len() - 1], t.width)])
        .filter(|&(at, width)| {
            along.is_point_closer_than(at, i64::from((stub.width + width) / 2) + 1)
        })
        .map(|(at, _)| {
            let t = (f64::from(at.x - pin.x) * dx + f64::from(at.y - pin.y) * dy) / length_squared;
            t.clamp(0.0, 1.0)
        })
        .reduce(f64::max);
    let Some(farthest) = farthest else {
        return Some(stub.clone());
    };
    let cut = Point::new(
        pin.x + (dx * farthest).round() as i32,
        pin.y + (dy * farthest).round() as i32,
    );
    (cut != pin).then(|| Track {
        path: vec![pin, cut],
        ..stub.clone()
    })
}

/// Escapes the pins of fine-pitch parts, part by part and along their rows,
/// each clear of the copper in `layout` and of the stubs before it, which are
/// added to `layout`. A pin with no room for a stub keeps its connections.
pub fn escape(
    design: &Design,
    rules: &ConstraintSet,
    plan: &Plan,
    layout: &mut impl LayoutAccess,
) -> Escaped {
    let mut escaped = Escaped {
        design: design.clone(),
        plan: plan.clone(),
        escapes: Vec::new(),
        pads_before: design.pads.len(),
    };
    for pin in fine_pitch_pins(design, plan) {
        let Some(stub) = stub_of(design, rules, layout, pin) else {
            continue;
        };
        let routes = Routes {
            tracks: vec![stub.clone()],
            vias: Vec::new(),
        };
        // Under an id of its own, as no connection owns it, only so that the
        // stubs after it keep clear of it.
        layout.add_route(ConnectionId::from_index(escaped.escapes.len()), &routes);
        let end = escaped.add_end(design.pad(pin), &stub);
        let part = design.pad(pin).part();
        let Escaped {
            design: so_far,
            plan,
            ..
        } = &mut escaped;
        for connection in &mut plan.connections {
            let leaves_the_part = |other: PadId| so_far.pad(other).part() != part;
            if connection.from == pin && leaves_the_part(connection.to) {
                connection.from = end;
            } else if connection.to == pin && leaves_the_part(connection.from) {
                connection.to = end;
            }
        }
        escaped.escapes.push(Escape { pin, stub });
    }
    escaped
}

/// The SMD pins, part by part and along their rows, of parts of at least
/// FINE_PITCH_PART_PINS pins, that copper of another net hems in, and that
/// connect to another part.
fn fine_pitch_pins(design: &Design, plan: &Plan) -> Vec<PadId> {
    let hemmed_in = super::hemmed_in_pads(design);
    let pins_of = |part: &str| design.pads.iter().filter(|p| p.part() == part).count();
    let connects_elsewhere = |pin: PadId| {
        let part = design.pad(pin).part();
        plan.connections.iter().any(|c| {
            (c.from == pin && design.pad(c.to).part() != part)
                || (c.to == pin && design.pad(c.from).part() != part)
        })
    };
    let mut pins: Vec<PadId> = (0..design.pads.len())
        .map(PadId::from_index)
        .filter(|&pin| {
            let pad = design.pad(pin);
            hemmed_in[pin.index()]
                && pad.copper.len() == 1
                && pad.net.is_some()
                && pins_of(pad.part()) >= FINE_PITCH_PART_PINS
                && connects_elsewhere(pin)
        })
        .collect();
    pins.sort_by_key(|&pin| {
        let pad = design.pad(pin);
        (
            pad.part(),
            pad.position.y,
            pad.position.x,
            pad.name.as_str(),
        )
    });
    pins
}

/// The shortest stub from `pin` away from its part that fits in `layout` and
/// ends off the pin, by a clearance, where a track may turn any way: straight
/// out first, then at 45° to either side.
fn stub_of(
    design: &Design,
    rules: &ConstraintSet,
    layout: &impl LayoutAccess,
    pin: PadId,
) -> Option<Track> {
    let pad = design.pad(pin);
    let net = pad.net?;
    let layer = pad.copper[0].layer;
    let width = rules.track_width(net);
    let clearance = rules.clearance(Some(net), Some(net));
    let step = (width / 2).max(1);
    let (out_x, out_y) = away_from_part(design, pin);
    let directions = [
        (out_x, out_y),
        (out_x - out_y, out_y + out_x),
        (out_x + out_y, out_y - out_x),
    ];
    directions.into_iter().find_map(|(dx, dy)| {
        (1..=2 * LONGEST_STUB_IN_TRACK_WIDTHS).find_map(|k| {
            let end = pad.position + Point::new(dx * step * k, dy * step * k);
            let is_off_the_pin = !pad.copper[0]
                .shape
                .is_closer_than(&Shape::circle(end, width), clearance);
            let may_turn_any_way =
                layout.is_track_clear(net, layer, Segment::new(end, end), width + 2 * clearance);
            let fits = layout.is_track_clear(net, layer, Segment::new(pad.position, end), width);
            (is_off_the_pin && may_turn_any_way && fits).then(|| Track {
                net,
                layer,
                width,
                path: vec![pad.position, end],
            })
        })
    })
}

/// The octilinear unit step square to the row of `pin`, the line to the
/// nearest other pin of its part, on the side away from the part's centre.
fn away_from_part(design: &Design, pin: PadId) -> (i32, i32) {
    let pad = design.pad(pin);
    let others: Vec<Point> = design
        .pads
        .iter()
        .filter(|p| p.part() == pad.part())
        .map(|p| p.position)
        .collect();
    let count = others.len() as i64;
    let centre =
        |axis: fn(Point) -> i32| others.iter().map(|&p| i64::from(axis(p))).sum::<i64>() / count;
    let outwards = (
        i64::from(pad.position.x) - centre(|p| p.x),
        i64::from(pad.position.y) - centre(|p| p.y),
    );
    let neighbour = others
        .iter()
        .filter(|&&p| p != pad.position)
        .min_by_key(|&&p| (p.distance(pad.position), p.x, p.y))
        .copied()
        .unwrap_or(pad.position);
    let along = (
        i64::from(neighbour.x - pad.position.x),
        i64::from(neighbour.y - pad.position.y),
    );
    let mut square = (-along.1, along.0);
    if square.0 * outwards.0 + square.1 * outwards.1 < 0 {
        square = (along.1, -along.0);
    }
    octilinear(square)
}

/// The nearest of the eight unit steps to the direction `(x, y)`.
fn octilinear((x, y): (i64, i64)) -> (i32, i32) {
    // tan 22.5° ≈ 0.414 ≈ 2 / 5: a component under that share of the other
    // rounds to zero.
    let step = |a: i64, b: i64| {
        if 5 * a.abs() >= 2 * b.abs() {
            a.signum() as i32
        } else {
            0
        }
    };
    (step(x, y), step(y, x))
}

#[cfg(test)]
mod tests {
    use super::*;
    use traxide_contracts::testing::{DesignBuilder, at, mm};
    use traxide_contracts::{Connection, Violation};
    use traxide_rules::Purpose;

    /// A chip of two rows of `pins` pads, 0.65 mm apart like a TSSOP's, rows
    /// 3 mm apart, each pin a net of its own that leads to a header pad far off.
    fn chip(pins: usize) -> DesignBuilder {
        let mut builder = DesignBuilder::new(40.0, 30.0)
            .track_width(0.2)
            .clearance(0.15);
        for i in 0..pins {
            let x = 10.0 + 0.65 * i as f64;
            for (row, y, far) in [(0, 8.5, 2.0), (1, 11.5, 28.0)] {
                let n = row * pins + i + 1;
                let net = format!("N{n}");
                builder = builder
                    .smd(&format!("U1-{n}"), Some(&net), at(x, y), 0.25, 0)
                    .tht(&format!("J1-{n}"), Some(&net), at(2.0 * n as f64, far), 1.0);
            }
        }
        builder
    }

    fn escaped(design: &Design) -> Escaped {
        let rules = traxide_rules::compile(
            design,
            Purpose::Routing {
                grid_pitch: 100_000,
            },
        );
        let mut layout = traxide_layout_access::Layout::build(design, &rules);
        escape(design, &rules, &crate::plan(design), &mut layout)
    }

    fn start_of(escaped: &Escaped, net: &str) -> traxide_contracts::Pad {
        let net = escaped.design.net_named(net);
        let connection = escaped
            .plan
            .connections
            .iter()
            .find(|c| c.net == net)
            .expect("the net has a connection");
        let pins: Vec<PadId> = [connection.from, connection.to]
            .into_iter()
            .filter(|&p| !escaped.design.pad(p).name.starts_with("J1"))
            .collect();
        escaped.design.pad(pins[0]).clone()
    }

    #[test]
    fn a_fine_pitch_pin_escapes_straight_away_from_its_part() {
        let design = chip(8).build();
        let escaped = escaped(&design);
        let pin = design.pad(design.pad_named("U1-1"));
        let pin_edge = pin.copper[0].shape.bounds().min.y;
        let start = start_of(&escaped, "N1").position;
        assert_eq!(start.x, pin.position.x);
        assert!(start.y < pin_edge, "{start:?} lies beyond the pin");
        assert_eq!(escaped.design.pads.len(), design.pads.len() + 16);
    }

    #[test]
    fn the_escape_is_a_stub_from_the_pin_to_where_its_connections_start() {
        let design = chip(8).build();
        let escaped = escaped(&design);
        let copper = escaped.copper_used(&[], &Routes::default());
        assert_eq!((copper.tracks.len(), copper.vias.len()), (16, 0));
        let start = start_of(&escaped, "N1").position;
        let pin = design.pad(design.pad_named("U1-1")).position;
        assert!(
            copper
                .tracks
                .iter()
                .any(|t| t.path == [pin, start] && t.net == design.net_named("N1"))
        );
    }

    #[test]
    fn a_stub_reaches_past_neighbours_too_close_for_a_track_to_turn() {
        // 0.4 mm apart, like a QFN's pins: a track only turns once it is
        // clear of both neighbours, by a clearance to spare.
        let mut builder = DesignBuilder::new(40.0, 30.0)
            .track_width(0.2)
            .clearance(0.15);
        for i in 0..8 {
            let x = 10.0 + 0.4 * i as f64;
            let net = format!("N{i}");
            builder = builder
                .smd(&format!("U1-{i}"), Some(&net), at(x, 8.5), 0.25, 0)
                .smd(&format!("U1-{}", i + 8), None, at(x, 11.5), 0.25, 0)
                .tht(
                    &format!("J1-{i}"),
                    Some(&net),
                    at(2.0 + 2.0 * i as f64, 2.0),
                    1.0,
                );
        }
        let design = builder.build();
        let escaped = escaped(&design);
        let pin = design.pad(design.pad_named("U1-3"));
        let start = start_of(&escaped, "N3").position;
        let neighbour = design.pad(design.pad_named("U1-2")).copper[0].shape.clone();
        let reach = mm(0.2 / 2.0 + 2.0 * 0.15);
        assert!(
            !neighbour.is_closer_than(&Shape::circle(start, 2 * reach), 0),
            "{start:?}"
        );
        assert!(start.y < pin.position.y);
    }

    #[test]
    fn a_stub_ends_where_the_route_farthest_from_its_pin_joins_it() {
        // The pad at a stub's end is all of the stub, so a route may join it
        // anywhere; beyond that the stub would lead nowhere.
        let design = chip(8).build();
        let escaped = escaped(&design);
        let n1 = design.net_named("N1");
        let pin = design.pad(design.pad_named("U1-1")).position;
        let start = start_of(&escaped, "N1").position;
        let middle = Point::new(pin.x, (pin.y + start.y) / 2);
        let route = Routes {
            tracks: vec![Track {
                net: n1,
                layer: traxide_contracts::LayerId(0),
                width: mm(0.2),
                path: vec![Point::new(middle.x - mm(2.0), middle.y), middle],
            }],
            vias: Vec::new(),
        };
        let copper = escaped.copper_used(&[], &route);
        let stub = copper
            .tracks
            .iter()
            .find(|t| t.net == n1)
            .expect("N1's stub");
        assert_eq!(stub.path, [pin, middle]);
    }

    #[test]
    fn escapes_keep_their_clearance_from_each_other_and_the_pins() {
        let design = chip(8).build();
        let escaped = escaped(&design);
        let rules = traxide_rules::compile(&design, Purpose::Verification);
        let verdict = traxide_verification::verify(
            &design,
            &rules,
            &escaped.copper_used(&[], &Routes::default()),
        );
        // Nothing is routed from the stubs yet, so they end in nothing.
        let broken: Vec<&Violation> = verdict
            .violations
            .iter()
            .filter(|v| {
                !matches!(
                    v,
                    Violation::Unconnected { .. } | Violation::Dangling { .. }
                )
            })
            .collect();
        assert!(broken.is_empty(), "{broken:?}");
    }

    #[test]
    fn pins_with_room_for_a_track_between_them_are_not_escaped() {
        let mut builder = DesignBuilder::new(40.0, 30.0)
            .track_width(0.2)
            .clearance(0.15);
        for n in 1..=8 {
            let net = format!("N{n}");
            builder = builder
                .smd(
                    &format!("U1-{n}"),
                    Some(&net),
                    at(1.27 * n as f64, 10.0),
                    0.6,
                    0,
                )
                .tht(
                    &format!("J1-{n}"),
                    Some(&net),
                    at(2.0 * n as f64, 25.0),
                    1.0,
                );
        }
        let design = builder.build();
        assert_eq!(escaped(&design).plan, crate::plan(&design));
    }

    #[test]
    fn the_pins_of_small_parts_are_not_escaped() {
        // A 0402 resistor: its two pads leave no room for a track between them.
        let design = DesignBuilder::new(30.0, 30.0)
            .track_width(0.2)
            .clearance(0.15)
            .smd("R1-1", Some("A"), at(10.0, 10.0), 0.5, 0)
            .smd("R1-2", Some("B"), at(10.9, 10.0), 0.5, 0)
            .tht("J1-1", Some("A"), at(2.0, 25.0), 1.0)
            .tht("J1-2", Some("B"), at(20.0, 25.0), 1.0)
            .build();
        assert_eq!(escaped(&design).plan, crate::plan(&design));
    }

    #[test]
    fn connections_between_pins_of_one_part_start_at_the_pins() {
        let design = chip(8).build();
        let mut design = design;
        // U1-1 and U1-3 share a net, across U1-2.
        let n1 = design.net_named("N1");
        let u1_3 = design.pad_named("U1-3");
        design.pads[u1_3.index()].net = Some(n1);
        design.nets[n1.index()].pads.push(u1_3);
        let escaped = escaped(&design);
        let between_pins = escaped.plan.connections.iter().find(|c| {
            let names = [c.from, c.to].map(|p| escaped.design.pad(p).name.clone());
            names.contains(&"U1-3".to_owned())
        });
        assert!(
            between_pins.is_some_and(|c: &Connection| {
                [c.from, c.to].contains(&design.pad_named("U1-1"))
            }),
            "{between_pins:?}"
        );
    }

    #[test]
    fn the_copper_of_an_escape_whose_connections_all_failed_is_left_out() {
        let design = chip(8).build();
        let escaped = escaped(&design);
        let n1 = escaped.design.net_named("N1");
        let failed: Vec<Connection> = escaped
            .plan
            .connections
            .iter()
            .copied()
            .filter(|c| c.net == n1)
            .collect();
        let copper = escaped.copper_used(&failed, &Routes::default());
        assert_eq!(copper.tracks.len(), 15);
        assert!(copper.tracks.iter().all(|t| t.net != n1));
    }

    #[test]
    fn a_stub_that_the_board_s_wiring_covers_is_left_out() {
        // A lane of N1's own leads out of U1-1 where its stub would.
        let mut design = chip(8).build();
        design.existing.tracks.push(traxide_contracts::Track {
            net: design.net_named("N1"),
            layer: traxide_contracts::LayerId(0),
            width: mm(0.2),
            path: vec![at(10.0, 8.5), at(10.0, 5.0)],
        });
        let escaped = escaped(&design);
        let n1 = escaped.design.net_named("N1");
        let copper = escaped.copper_used(&[], &Routes::default());
        assert_eq!(copper.tracks.len(), 15);
        assert!(copper.tracks.iter().all(|t| t.net != n1));
    }

    #[test]
    fn a_connection_between_two_fine_pitch_parts_runs_from_escape_to_escape() {
        let mut builder = DesignBuilder::new(40.0, 40.0)
            .track_width(0.2)
            .clearance(0.15);
        for part in ["U1", "U2"] {
            let y = if part == "U1" { 8.5 } else { 30.0 };
            for i in 0..8 {
                let x = 10.0 + 0.65 * i as f64;
                let net = format!("N{i}");
                builder = builder
                    .smd(&format!("{part}-{i}"), Some(&net), at(x, y), 0.25, 0)
                    .smd(&format!("{part}-{}", i + 8), None, at(x, y + 3.0), 0.25, 0);
            }
        }
        let design = builder.build();
        let escaped = escaped(&design);
        let n0 = escaped.design.net_named("N0");
        let connection = escaped.plan.connections.iter().find(|c| c.net == n0);
        assert!(
            connection.is_some_and(|c| {
                let names = [c.from, c.to].map(|p| escaped.design.pad(p).name.clone());
                names.iter().all(|n| n.ends_with("~escape"))
            }),
            "{connection:?}"
        );
    }

    #[test]
    fn a_connection_from_an_escape_reports_as_one_from_its_pin() {
        let design = chip(8).build();
        let escaped = escaped(&design);
        let n1 = escaped.design.net_named("N1");
        let connection = escaped.plan.connections.iter().find(|c| c.net == n1);
        let pins = connection
            .map(|&c| escaped.at_pins(c))
            .map(|c| [c.from, c.to]);
        assert_eq!(
            pins,
            Some([design.pad_named("J1-1"), design.pad_named("U1-1")])
        );
    }
}
