use super::project::{Project, nanometres};
use super::{rules, sexpr};
use crate::BoardError;
use crate::sexp::Sexp;
use std::collections::HashMap;
use traxide_contracts::geometry::{Point, Polygon, Rect, Region, Segment, Shape, Transform};
use traxide_contracts::{
    Design, DesignRules, Keepout, KeepoutScope, Layer, LayerId, LayerShape, Net, NetClass,
    NetClassId, NetId, Pad, Plane, Routes, Track, Via, ViaKind, ViaKindId,
};

/// A board as read, with what storing routes into its file needs beyond the
/// design.
pub(crate) struct Board {
    pub design: Design,
    /// The size of each via kind, by the kind's index.
    pub via_sizes: Vec<ViaSize>,
    /// KiCad 9 refers to nets by number: each net's number by its name. KiCad
    /// 10 refers to them by name.
    pub net_numbers: Option<HashMap<String, String>>,
    /// Whether the board has zones of copper, which KiCad fills around routes.
    pub has_copper_zones: bool,
}

/// The board in `text`, with the rules of its project and its custom rules.
pub(crate) fn read(
    text: &str,
    project: Option<&str>,
    custom_rules: Option<&str>,
) -> Result<Board, BoardError> {
    read_with(text, Project::read(project)?, custom_rules)
}

/// The board in `text`, with the rules of `project` and its custom rules.
pub(crate) fn read_with(
    text: &str,
    project: Project,
    custom_rules: Option<&str>,
) -> Result<Board, BoardError> {
    let root = sexpr::parse(text.trim_start_matches('\u{feff}'))?;
    if root.keyword() != Some("kicad_pcb") {
        return Err(root.invalid("not a KiCad board: expected (kicad_pcb …)"));
    }
    Reader::new(&root, project)?.read(custom_rules)
}

/// How far an arc may bulge beyond the straight segments it is read as.
const ARC_TOLERANCE: f64 = 2_000.0;
/// How close the ends of two edges must be to join them into an outline.
const EDGE_JOIN_DISTANCE: i64 = 1_000;
const EDGE_CUTS: &str = "Edge.Cuts";

struct Reader<'a> {
    root: &'a Sexp,
    project: Project,
    layers: Vec<Layer>,
    layer_by_name: HashMap<&'a str, LayerId>,
    /// KiCad 9's list of nets: each net's name by its number.
    numbered_nets: Option<HashMap<&'a str, &'a str>>,
    nets: Vec<Net>,
    net_by_name: HashMap<&'a str, NetId>,
    via_kinds: ViaKinds,
    keepouts: Vec<Keepout>,
    /// The drawings on the Edge.Cuts layer, each open or closed, with the
    /// reference of the footprint that draws it.
    edges: Vec<Edge<'a>>,
    /// The references of the footprints read so far.
    footprints: Vec<&'a str>,
    has_copper_zones: bool,
    /// The copper zones read so far: net name, layer, outline.
    zones: Vec<Zone<'a>>,
}

impl<'a> Reader<'a> {
    fn new(root: &'a Sexp, project: Project) -> Result<Self, BoardError> {
        let listed = root.required_child("layers")?;
        let copper: Vec<(&str, &str)> = listed
            .args()
            .iter()
            .filter_map(|layer| {
                let name = layer.args().first()?.atom()?;
                let kind = layer.args().get(1).and_then(Sexp::atom).unwrap_or("signal");
                name.ends_with(".Cu").then_some((name, kind))
            })
            .collect();
        // KiCad lists the copper layers as they are stacked, but numbers them
        // differently in each version; the front is on top and the back below.
        let (outer, inner): (Vec<_>, Vec<_>) = copper
            .into_iter()
            .partition(|(name, _)| *name == "F.Cu" || *name == "B.Cu");
        let mut stacked: Vec<(&str, &str)> =
            outer.iter().copied().filter(|l| l.0 == "F.Cu").collect();
        stacked.extend(inner);
        stacked.extend(outer.iter().copied().filter(|l| l.0 == "B.Cu"));
        if stacked.is_empty() {
            return Err(listed.invalid("the board has no copper layers"));
        }
        let numbered: Vec<&Sexp> = root.children("net").collect();
        let numbered_nets = (!numbered.is_empty()).then(|| {
            numbered
                .iter()
                .filter_map(|net| Some((net.args().first()?.atom()?, net.args().get(1)?.atom()?)))
                .collect()
        });
        Ok(Self {
            root,
            project,
            layer_by_name: stacked
                .iter()
                .enumerate()
                .map(|(i, (name, _))| (*name, LayerId::from_index(i)))
                .collect(),
            layers: stacked
                .iter()
                .map(|(name, kind)| Layer {
                    name: (*name).to_owned(),
                    routable: *kind != "power",
                })
                .collect(),
            numbered_nets,
            nets: Vec::new(),
            net_by_name: HashMap::new(),
            via_kinds: ViaKinds::default(),
            keepouts: Vec::new(),
            edges: Vec::new(),
            footprints: Vec::new(),
            has_copper_zones: false,
            zones: Vec::new(),
        })
    }

    fn read(mut self, custom_rules: Option<&str>) -> Result<Board, BoardError> {
        // Nets numbered in the board keep their order.
        for net in self.root.children("net") {
            if let Some(name) = net.args().get(1).and_then(Sexp::atom)
                && !name.is_empty()
            {
                self.net(name);
            }
        }
        let mut rules = self.rules();
        let mut pads = Vec::new();
        for footprint in self.root.children("footprint") {
            self.footprint(footprint, &mut pads)?;
        }
        let (existing, locked) = self.existing()?;
        for zone in self.root.children("zone") {
            self.zone(zone)?;
        }
        for item in self.root.args() {
            if let Some((points, closed)) = drawing(item, "gr_", &Transform::IDENTITY)? {
                self.edges.push(Edge {
                    points,
                    closed,
                    footprint: None,
                });
            }
        }
        let (outline, cutout_footprints) = self.outline()?;
        let mut warnings = Vec::new();
        if let Some(text) = custom_rules {
            let names = rules::Names {
                classes: &rules.classes,
                nets: &self.nets,
                layers: &self.layers,
                footprints: &self.footprints,
            };
            let read = rules::read(text, &names);
            rules.custom = read.rules;
            warnings = read.warnings;
        }
        for (index, pad) in pads.iter().enumerate() {
            if let Some(net) = pad.net {
                self.nets[net.index()]
                    .pads
                    .push(traxide_contracts::PadId::from_index(index));
            }
        }
        let net_numbers = self.numbered_nets.as_ref().map(|numbers| {
            numbers
                .iter()
                .map(|(number, name)| ((*name).to_owned(), (*number).to_owned()))
                .collect()
        });
        let planes = self.planes();
        // KiCad routes on a layer of type power like on any other unless a
        // zone fills it.
        for (index, layer) in self.layers.iter_mut().enumerate() {
            let holds_a_zone = self.zones.iter().any(|zone| zone.layer.index() == index);
            layer.routable |= !holds_a_zone;
        }
        Ok(Board {
            design: Design {
                planes,
                layers: self.layers,
                outline,
                cutout_footprints,
                locked,
                nets: self.nets,
                pads,
                via_kinds: self.via_kinds.kinds,
                keepouts: self.keepouts,
                existing,
                rules,
                // KiCad's files are in millimetres to the nanometre.
                resolution: 1,
                outline_tolerance: ARC_TOLERANCE as i32,
                warnings,
            },
            via_sizes: self.via_kinds.sizes,
            net_numbers,
            has_copper_zones: self.has_copper_zones,
        })
    }

    fn all_layers(&self) -> Vec<LayerId> {
        (0..self.layers.len()).map(LayerId::from_index).collect()
    }

    fn rules(&mut self) -> DesignRules {
        let all = self.all_layers();
        let classes = self
            .project
            .classes
            .clone()
            .into_iter()
            .map(|class| NetClass {
                via_kinds: vec![
                    self.via_kinds
                        .through(&all, class.via_diameter, class.via_drill),
                ],
                name: class.name,
                track_width: class.track_width,
                clearance: class.clearance,
            })
            .collect();
        DesignRules {
            classes,
            default_class: NetClassId::from_index(0),
            edge_clearance: self.project.edge_clearance,
            hole_clearance: self.project.hole_clearance,
            hole_to_hole: self.project.hole_to_hole,
            custom: Vec::new(),
        }
    }

    /// The net named `name`, added when first met.
    fn net(&mut self, name: &'a str) -> NetId {
        if let Some(&id) = self.net_by_name.get(name) {
            return id;
        }
        let id = NetId::from_index(self.nets.len());
        self.nets.push(Net {
            name: name.to_owned(),
            class: NetClassId::from_index(self.project.class_of(name)),
            pads: Vec::new(),
        });
        self.net_by_name.insert(name, id);
        id
    }

    /// The net `item` belongs to: `(net "name")` in KiCad 10, `(net 3)` or
    /// `(net 3 "name")` in KiCad 9. `None` for copper of no net.
    fn net_of(&mut self, item: &'a Sexp) -> Option<NetId> {
        let name = self.net_name_of(item)?;
        Some(self.net(name))
    }

    fn net_name_of(&self, item: &'a Sexp) -> Option<&'a str> {
        let net = item.child("net")?;
        let first = net.args().first()?.atom()?;
        let name = match (net.args().get(1).and_then(Sexp::atom), &self.numbered_nets) {
            (Some(name), _) => name,
            (None, Some(numbers)) => numbers.get(first).copied()?,
            (None, None) => first,
        };
        (!name.is_empty()).then_some(name)
    }

    /// The copper layers among the layer names of `item`.
    fn copper_layers(&self, item: &Sexp) -> Vec<LayerId> {
        let names = item
            .child("layers")
            .or_else(|| item.child("layer"))
            .map(|l| l.atoms().collect::<Vec<_>>())
            .unwrap_or_default();
        let mut layers: Vec<LayerId> = names
            .iter()
            .flat_map(|name| match *name {
                "*.Cu" => self.all_layers(),
                "F&B.Cu" => vec![
                    LayerId::from_index(0),
                    LayerId::from_index(self.layers.len() - 1),
                ],
                name => self.layer_by_name.get(name).copied().into_iter().collect(),
            })
            .collect();
        layers.sort();
        layers.dedup();
        layers
    }

    fn keepouts_on(&mut self, layers: &[LayerId], shape: Shape, scope: KeepoutScope) {
        if layers.len() == self.layers.len() {
            self.keepouts.push(Keepout {
                layer: None,
                shape,
                scope,
                clearance: 0,
            });
        } else {
            for &layer in layers {
                self.keepouts.push(Keepout {
                    layer: Some(layer),
                    shape: shape.clone(),
                    scope,
                    clearance: 0,
                });
            }
        }
    }

    fn footprint(&mut self, footprint: &'a Sexp, pads: &mut Vec<Pad>) -> Result<(), BoardError> {
        let at = footprint.required_child("at")?;
        let placement = placed_at(at)?;
        let reference = footprint
            .children("property")
            .chain(footprint.children("fp_text"))
            .find(|p| {
                p.args()
                    .first()
                    .and_then(Sexp::atom)
                    .is_some_and(|key| key.eq_ignore_ascii_case("reference"))
            })
            .and_then(|p| p.args().get(1)?.atom())
            .unwrap_or_default();
        self.footprints.push(reference);
        for pad in footprint.children("pad") {
            if let Some(read) = self.pad(pad, &placement)? {
                let base = format!("{reference}-{}", pad.arg_atom(0, "pad number")?);
                let mut name = base.clone();
                let mut repeat = 1;
                while pads.iter().any(|p: &Pad| p.name == name) {
                    name = format!("{base}@{repeat}");
                    repeat += 1;
                }
                pads.push(Pad { name, ..read });
            }
        }
        for item in footprint.args() {
            if let Some((points, closed)) = drawing(item, "fp_", &placement)? {
                self.edges.push(Edge {
                    points,
                    closed,
                    footprint: Some(reference),
                });
            }
        }
        for zone in footprint.children("zone") {
            self.zone(zone)?;
        }
        Ok(())
    }

    /// The pad `pad` of a footprint placed by `placement`; `None` for a pad
    /// without copper, whose hole becomes a keepout.
    fn pad(&mut self, pad: &'a Sexp, placement: &Transform) -> Result<Option<Pad>, BoardError> {
        let kind = pad.arg_atom(1, "pad type")?;
        let form = pad.arg_atom(2, "pad shape")?;
        let at = pad.required_child("at")?;
        let position = placement.apply(point(at)?);
        // A pad's angle in the board file includes its footprint's.
        let oriented = Transform {
            origin: position,
            ..placed_at(at)?
        };
        let drill = pad.child("drill");
        if kind == "np_thru_hole" {
            if let Some(drill) = drill {
                let all = self.all_layers();
                // As KiCad's DSN export does: the hole, widened by the clearance
                // that KiCad's DRC keeps copper from holes.
                let hole = match hole(drill)? {
                    Shape::Capsule { segment, radius } => Shape::Capsule {
                        segment,
                        radius: radius + self.project.hole_clearance,
                    },
                    polygon => polygon,
                };
                let hole = hole.map(|p| oriented.apply(p));
                self.keepouts_on(&all, hole, KeepoutScope::Everything);
            }
            return Ok(None);
        }
        let layers = self.copper_layers(pad);
        if layers.is_empty() {
            return Ok(None);
        }
        let size = pad.required_child("size")?;
        let (width, height) = (millimetres(size, 0)?, millimetres(size, 1)?);
        let offset = match drill.and_then(|d| d.child("offset")) {
            Some(offset) => point(offset)?,
            None => Point::default(),
        };
        let shapes = match form {
            "circle" => vec![Shape::circle(offset, width)],
            "oval" => vec![stadium(offset, width, height)],
            "rect" => vec![rectangle(offset, width, height)],
            "trapezoid" => {
                // The bounding rectangle: trapezoids are rare, and it covers them.
                let delta = match pad.child("rect_delta") {
                    Some(delta) => millimetres(delta, 0)?
                        .abs()
                        .max(millimetres(delta, 1)?.abs()),
                    None => 0,
                };
                vec![rectangle(offset, width + delta, height + delta)]
            }
            "roundrect" => {
                let ratio = match pad.child("roundrect_rratio") {
                    Some(r) => number(r, 0, "corner ratio")?,
                    None => 0.0,
                };
                let chamfered = pad
                    .child("chamfer_ratio")
                    .is_some_and(|c| number(c, 0, "chamfer ratio").is_ok_and(|r| r > 0.0));
                if chamfered {
                    vec![rectangle(offset, width, height)]
                } else {
                    let radius = ratio * f64::from(width.min(height));
                    vec![rounded_rectangle(offset, width, height, radius)]
                }
            }
            "custom" => {
                let anchor = pad
                    .child("options")
                    .and_then(|o| o.child("anchor"))
                    .and_then(|a| a.atoms().next());
                let mut shapes = vec![if anchor == Some("circle") {
                    Shape::circle(offset, width)
                } else {
                    rectangle(offset, width, height)
                }];
                if let Some(primitives) = pad.child("primitives") {
                    for primitive in primitives.args() {
                        shapes.extend(primitive_shapes(primitive)?);
                    }
                }
                shapes
            }
            other => return Err(pad.invalid(format!("unsupported pad shape {other}"))),
        };
        let copper = layers
            .iter()
            .flat_map(|&layer| {
                shapes.iter().map(move |shape| LayerShape {
                    layer,
                    shape: shape.map(|p| oriented.apply(p)),
                })
            })
            .collect();
        Ok(Some(Pad {
            name: String::new(),
            net: self.net_of(pad),
            position,
            copper,
        }))
    }

    /// A rule area becomes keepouts; copper zones are left out, as KiCad
    /// refills them around the routes.
    fn zone(&mut self, zone: &'a Sexp) -> Result<(), BoardError> {
        let Some(rules) = zone.child("keepout") else {
            self.has_copper_zones = true;
            return self.copper_zone(zone);
        };
        let forbids =
            |what: &str| rules.child(what).and_then(|c| c.atoms().next()) == Some("not_allowed");
        let scope = match (forbids("tracks"), forbids("vias")) {
            (true, true) => KeepoutScope::Everything,
            (true, false) => KeepoutScope::TracksOnly,
            (false, true) => KeepoutScope::ViasOnly,
            (false, false) => return Ok(()),
        };
        let layers = self.copper_layers(zone);
        for outline in zone.children("polygon") {
            let points = points(outline.required_child("pts")?)?;
            let polygon = Polygon::new(distinct(points))
                .ok_or_else(|| outline.invalid("a keepout needs at least three corners"))?;
            self.keepouts_on(&layers, Shape::Polygon(polygon), scope);
        }
        Ok(())
    }

    /// A copper zone of a net, as a plane on each of its layers; the net is
    /// known by name until every pad is read.
    fn copper_zone(&mut self, zone: &'a Sexp) -> Result<(), BoardError> {
        let Some(net) = self.net_name_of(zone) else {
            return Ok(());
        };
        let layers = self.copper_layers(zone);
        let setting = |within: Option<&Sexp>, keyword: &str| {
            within
                .and_then(|w| w.child(keyword))
                .map_or(Ok(0), |s| millimetres(s, 0))
        };
        let clearance = setting(zone.child("connect_pads"), "clearance")?;
        let min_width = setting(Some(zone), "min_thickness")?;
        for outline in zone.children("polygon") {
            let points = points(outline.required_child("pts")?)?;
            let Some(polygon) = Polygon::new(distinct(points)) else {
                continue;
            };
            for &layer in &layers {
                self.zones.push(Zone {
                    net,
                    layer,
                    outline: polygon.clone(),
                    clearance,
                    min_width,
                });
            }
        }
        Ok(())
    }

    /// The zones of nets that pads are in, as planes.
    fn planes(&self) -> Vec<Plane> {
        self.zones
            .iter()
            .filter_map(|zone| {
                let net = *self.net_by_name.get(zone.net)?;
                (!self.nets[net.index()].pads.is_empty()).then(|| Plane {
                    net,
                    layer: zone.layer,
                    outline: zone.outline.clone(),
                    holes: Vec::new(),
                    clearance: zone.clearance,
                    min_width: zone.min_width,
                })
            })
            .collect()
    }

    /// The board's own tracks and vias, and those of them the designer
    /// locked. Copper of no net cannot be a route and keeps other copper away
    /// instead.
    fn existing(&mut self) -> Result<(Routes, Routes), BoardError> {
        let (mut routes, mut locked) = (Routes::default(), Routes::default());
        for item in self.root.args() {
            let path = match item.keyword() {
                Some("segment") => vec![
                    point(item.required_child("start")?)?,
                    point(item.required_child("end")?)?,
                ],
                Some("arc") => arc(
                    point(item.required_child("start")?)?,
                    point(item.required_child("mid")?)?,
                    point(item.required_child("end")?)?,
                ),
                Some("via") => {
                    let before = routes.vias.len();
                    self.via(item, &mut routes)?;
                    if is_locked(item) && routes.vias.len() > before {
                        locked.vias.extend(routes.vias.last().cloned());
                    }
                    continue;
                }
                _ => continue,
            };
            let width = millimetres(item.required_child("width")?, 0)?;
            let Some(&layer) = self.copper_layers(item).first() else {
                continue;
            };
            match self.net_of(item) {
                Some(net) => {
                    let track = Track {
                        net,
                        layer,
                        width,
                        path,
                    };
                    if is_locked(item) {
                        locked.tracks.push(track.clone());
                    }
                    routes.tracks.push(track);
                }
                None => {
                    for pair in path.windows(2) {
                        let shape = Shape::track(Segment::new(pair[0], pair[1]), width);
                        self.keepouts_on(&[layer], shape, KeepoutScope::Everything);
                    }
                }
            }
        }
        Ok((routes, locked))
    }

    fn via(&mut self, via: &'a Sexp, routes: &mut Routes) -> Result<(), BoardError> {
        let position = point(via.required_child("at")?)?;
        let diameter = millimetres(via.required_child("size")?, 0)?;
        let drill = match via.child("drill") {
            Some(drill) => millimetres(drill, 0)?,
            None => 0,
        };
        let ends = self.copper_layers(via);
        let (Some(&top), Some(&bottom)) = (ends.first(), ends.last()) else {
            return Err(via.invalid("a via needs its copper layers"));
        };
        let span: Vec<LayerId> = (top.index()..=bottom.index())
            .map(LayerId::from_index)
            .collect();
        match self.net_of(via) {
            Some(net) => {
                let kind = self.via_kinds.through(&span, diameter, drill);
                routes.vias.push(Via {
                    net,
                    kind,
                    position,
                });
            }
            None => self.keepouts_on(
                &span,
                Shape::circle(position, diameter),
                KeepoutScope::Everything,
            ),
        }
        Ok(())
    }

    /// The outer loops of the edges are the board; loops inside them are
    /// holes in it, cutouts that copper keeps the edge clearance from, as
    /// KiCad's DRC does, whether the board or a footprint draws them.
    /// Also the footprint that draws each hole, in their order.
    fn outline(&mut self) -> Result<(Region, Vec<Option<String>>), BoardError> {
        let polygons: Vec<(Polygon, Option<&str>)> = loops(std::mem::take(&mut self.edges))
            .into_iter()
            .filter_map(|(points, footprint)| Some((Polygon::new(distinct(points))?, footprint)))
            .collect();
        let (mut outer, mut holes, mut owners) = (Vec::new(), Vec::new(), Vec::new());
        for (i, (polygon, footprint)) in polygons.iter().enumerate() {
            let around = polygons
                .iter()
                .enumerate()
                .filter(|&(j, (other, _))| j != i && other.contains(polygon.points()[0]))
                .count();
            if around % 2 == 0 {
                outer.push(polygon.clone());
            } else {
                holes.push(polygon.clone());
                owners.push(footprint.map(str::to_owned));
            }
        }
        Region::new(outer)
            .map(|region| (region.with_holes(holes), owners))
            .ok_or_else(|| {
                self.root
                    .invalid("the board has no closed outline on the Edge.Cuts layer")
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ViaSize {
    pub diameter: i32,
    pub drill: i32,
}

#[derive(Default)]
struct ViaKinds {
    kinds: Vec<ViaKind>,
    sizes: Vec<ViaSize>,
}

impl ViaKinds {
    /// The kind of via of `diameter` and `drill` through the layers of `span`.
    fn through(&mut self, span: &[LayerId], diameter: i32, drill: i32) -> ViaKindId {
        let name = format!(
            "Via[{}-{}]_{}:{}_um",
            span[0].index(),
            span[span.len() - 1].index(),
            f64::from(diameter) / 1000.0,
            f64::from(drill) / 1000.0
        );
        if let Some(index) = self.kinds.iter().position(|k| k.name == name) {
            return ViaKindId::from_index(index);
        }
        self.kinds.push(ViaKind {
            name,
            copper: span
                .iter()
                .map(|&layer| LayerShape {
                    layer,
                    shape: Shape::circle(Point::default(), diameter),
                })
                .collect(),
            drill: Some(drill),
        });
        self.sizes.push(ViaSize { diameter, drill });
        ViaKindId::from_index(self.kinds.len() - 1)
    }
}

fn number(sexp: &Sexp, index: usize, what: &str) -> Result<f64, BoardError> {
    let text = sexp.arg_atom(index, what)?;
    text.parse()
        .map_err(|_| sexp.invalid(format!("{what} is not a number: {text}")))
}

fn millimetres(sexp: &Sexp, index: usize) -> Result<i32, BoardError> {
    Ok(nanometres(number(sexp, index, "length")?))
}

/// `(xy x y)`, `(start x y)`, `(at x y angle)` and the like.
fn point(sexp: &Sexp) -> Result<Point, BoardError> {
    Ok(Point::new(millimetres(sexp, 0)?, millimetres(sexp, 1)?))
}

fn points(pts: &Sexp) -> Result<Vec<Point>, BoardError> {
    let mut points = Vec::new();
    for item in pts.args() {
        match item.keyword() {
            Some("xy") => points.push(point(item)?),
            Some("arc") => {
                let arc = arc(
                    point(item.required_child("start")?)?,
                    point(item.required_child("mid")?)?,
                    point(item.required_child("end")?)?,
                );
                points.extend(arc);
            }
            _ => {}
        }
    }
    Ok(points)
}

/// Where `(at x y angle)` places things. KiCad's y axis points down, so its
/// counter-clockwise angles turn the other way than the transform's.
fn placed_at(at: &Sexp) -> Result<Transform, BoardError> {
    let degrees = match at.args().get(2) {
        Some(_) => number(at, 2, "angle")?,
        None => 0.0,
    };
    Ok(Transform {
        origin: point(at)?,
        rotation_degrees: -degrees,
        mirror_x: false,
    })
}

/// `(drill d)` or `(drill oval w h)`, centred on the pad.
fn hole(drill: &Sexp) -> Result<Shape, BoardError> {
    if drill.args().first().and_then(Sexp::atom) == Some("oval") {
        Ok(stadium(
            Point::default(),
            millimetres(drill, 1)?,
            millimetres(drill, 2)?,
        ))
    } else {
        Ok(Shape::circle(Point::default(), millimetres(drill, 0)?))
    }
}

fn rectangle(centre: Point, width: i32, height: i32) -> Shape {
    let half = Point::new(width / 2, height / 2);
    Shape::Polygon(Polygon::rectangle(Rect {
        min: centre - half,
        max: centre + Point::new(width - half.x, height - half.y),
    }))
}

/// A rectangle with corners of `radius`, cut off at 45° where the corners touch
/// it: it covers the rounded corners and is barely larger.
fn rounded_rectangle(centre: Point, width: i32, height: i32, radius: f64) -> Shape {
    let cut = (radius * (2.0 - std::f64::consts::SQRT_2)).round() as i32;
    if cut <= 0 {
        return rectangle(centre, width, height);
    }
    let (left, top) = (centre.x - width / 2, centre.y - height / 2);
    let (right, bottom) = (left + width, top + height);
    let corners = vec![
        Point::new(left + cut, top),
        Point::new(right - cut, top),
        Point::new(right, top + cut),
        Point::new(right, bottom - cut),
        Point::new(right - cut, bottom),
        Point::new(left + cut, bottom),
        Point::new(left, bottom - cut),
        Point::new(left, top + cut),
    ];
    Polygon::new(corners).map_or_else(|| rectangle(centre, width, height), Shape::Polygon)
}

/// A rectangle with round ends on its shorter sides.
fn stadium(centre: Point, width: i32, height: i32) -> Shape {
    let half_length = (width - height).abs() / 2;
    let along = if width > height {
        Point::new(half_length, 0)
    } else {
        Point::new(0, half_length)
    };
    Shape::track(
        Segment::new(centre - along, centre + along),
        width.min(height),
    )
}

/// The shapes a custom pad adds to its anchor, relative to the pad.
fn primitive_shapes(primitive: &Sexp) -> Result<Vec<Shape>, BoardError> {
    let width = match primitive.child("width") {
        Some(width) => millimetres(width, 0)?,
        None => 0,
    };
    Ok(match primitive.keyword() {
        Some("gr_poly") => {
            let points = distinct(points(primitive.required_child("pts")?)?);
            Polygon::new(points)
                .map(Shape::Polygon)
                .into_iter()
                .collect()
        }
        Some("gr_circle") => {
            let centre = point(primitive.required_child("center")?)?;
            let end = point(primitive.required_child("end")?)?;
            let radius = i32::try_from(centre.distance(end)).unwrap_or(i32::MAX / 4);
            vec![Shape::circle(centre, 2 * radius + width)]
        }
        Some("gr_rect") => {
            let corners = [
                point(primitive.required_child("start")?)?,
                point(primitive.required_child("end")?)?,
            ];
            let rect = Rect::around(corners).unwrap_or_else(|| unreachable!("two corners"));
            vec![Shape::Polygon(Polygon::rectangle(rect.inflated(width / 2)))]
        }
        Some("gr_line") => vec![Shape::track(
            Segment::new(
                point(primitive.required_child("start")?)?,
                point(primitive.required_child("end")?)?,
            ),
            width,
        )],
        Some("gr_arc") => {
            let path = arc(
                point(primitive.required_child("start")?)?,
                point(primitive.required_child("mid")?)?,
                point(primitive.required_child("end")?)?,
            );
            path.windows(2)
                .map(|pair| Shape::track(Segment::new(pair[0], pair[1]), width))
                .collect()
        }
        _ => Vec::new(),
    })
}

/// A drawing on the Edge.Cuts layer, with `prefix` gr_ on the board and fp_ in
/// a footprint: its points, and whether it is closed.
fn drawing(
    item: &Sexp,
    prefix: &str,
    placement: &Transform,
) -> Result<Option<(Vec<Point>, bool)>, BoardError> {
    let Some(form) = item.keyword().and_then(|k| k.strip_prefix(prefix)) else {
        return Ok(None);
    };
    let on_edge = item
        .child("layer")
        .and_then(|l| l.atoms().next())
        .is_some_and(|layer| layer == EDGE_CUTS);
    if !on_edge {
        return Ok(None);
    }
    let at = |keyword: &str| item.required_child(keyword).and_then(point);
    let (points, closed) = match form {
        "line" => (vec![at("start")?, at("end")?], false),
        "arc" => (arc(at("start")?, at("mid")?, at("end")?), false),
        // A Bézier curve, by its control polygon: edges are rarely curved.
        "curve" => (points(item.required_child("pts")?)?, false),
        "rect" => {
            let (a, b) = (at("start")?, at("end")?);
            (vec![a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)], true)
        }
        "circle" => (circle(at("center")?, at("end")?), true),
        "poly" => (points(item.required_child("pts")?)?, true),
        _ => return Ok(None),
    };
    Ok(Some((
        points.into_iter().map(|p| placement.apply(p)).collect(),
        closed,
    )))
}

/// The points of an arc from `start` through `mid` to `end`, close enough
/// that no part of the arc is more than ARC_TOLERANCE from them.
/// Whether the designer locked a track or via: `(locked yes)`, or `locked`
/// among its words as KiCad wrote it before version 7.
fn is_locked(item: &Sexp) -> bool {
    item.atoms().any(|word| word == "locked")
        || item
            .child("locked")
            .is_some_and(|locked| locked.atoms().next().is_none_or(|word| word == "yes"))
}

fn arc(start: Point, mid: Point, end: Point) -> Vec<Point> {
    let [(ax, ay), (bx, by), (cx, cy)] =
        [start, mid, end].map(|p| (f64::from(p.x), f64::from(p.y)));
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    if d.abs() < f64::EPSILON {
        return vec![start, end];
    }
    let square = |x: f64, y: f64| x * x + y * y;
    let ux =
        (square(ax, ay) * (by - cy) + square(bx, by) * (cy - ay) + square(cx, cy) * (ay - by)) / d;
    let uy =
        (square(ax, ay) * (cx - bx) + square(bx, by) * (ax - cx) + square(cx, cy) * (bx - ax)) / d;
    let radius = square(ax - ux, ay - uy).sqrt();
    let angle = |x: f64, y: f64| (y - uy).atan2(x - ux);
    let turn = std::f64::consts::TAU;
    let (from, through, to) = (angle(ax, ay), angle(bx, by), angle(cx, cy));
    let sweep_to = (to - from).rem_euclid(turn);
    let sweep = if (through - from).rem_euclid(turn) < sweep_to {
        sweep_to
    } else {
        sweep_to - turn
    };
    let steps = steps_for(radius, sweep.abs());
    let mut points = vec![start];
    points.extend((1..steps).map(|i| {
        let a = from + sweep * f64::from(i) / f64::from(steps);
        Point::new(
            (ux + radius * a.cos()).round() as i32,
            (uy + radius * a.sin()).round() as i32,
        )
    }));
    points.push(end);
    points
}

fn circle(centre: Point, on: Point) -> Vec<Point> {
    let radius = centre.distance(on) as f64;
    let steps = steps_for(radius, std::f64::consts::TAU);
    (0..steps)
        .map(|i| {
            let a = std::f64::consts::TAU * f64::from(i) / f64::from(steps);
            Point::new(
                centre.x + (radius * a.cos()).round() as i32,
                centre.y + (radius * a.sin()).round() as i32,
            )
        })
        .collect()
}

/// How many straight steps follow an arc of `radius` through `sweep` radians
/// within ARC_TOLERANCE.
fn steps_for(radius: f64, sweep: f64) -> u32 {
    const MOST: f64 = 720.0;
    let step = if radius > ARC_TOLERANCE {
        2.0 * (1.0 - ARC_TOLERANCE / radius).acos()
    } else {
        std::f64::consts::FRAC_PI_2
    };
    (sweep / step).ceil().clamp(2.0, MOST) as u32
}

/// A copper zone, its net known by name until every pad is read.
struct Zone<'a> {
    net: &'a str,
    layer: LayerId,
    outline: Polygon,
    clearance: i32,
    min_width: i32,
}

/// A drawing on the Edge.Cuts layer.
struct Edge<'a> {
    points: Vec<Point>,
    closed: bool,
    /// The reference of the footprint that draws it; `None` on the board.
    footprint: Option<&'a str>,
}

/// Joins open drawings end to end into closed loops; drawings that close no
/// loop are left out, as KiCad's DRC reports them. A loop belongs to a
/// footprint when all its drawings do.
fn loops(edges: Vec<Edge<'_>>) -> Vec<(Vec<Point>, Option<&str>)> {
    let near = |a: Point, b: Point| a.distance(b) <= EDGE_JOIN_DISTANCE;
    let (closed, mut open): (Vec<_>, Vec<_>) = edges.into_iter().partition(|edge| edge.closed);
    let mut loops: Vec<(Vec<Point>, Option<&str>)> = closed
        .into_iter()
        .map(|edge| (edge.points, edge.footprint))
        .collect();
    open.reverse();
    while let Some(Edge {
        points: mut chain,
        mut footprint,
        ..
    }) = open.pop()
    {
        loop {
            let (first, last) = (chain[0], chain[chain.len() - 1]);
            if chain.len() > 3 && near(first, last) {
                chain.pop();
                loops.push((chain, footprint));
                break;
            }
            let Some(next) = open.iter().rposition(|edge| {
                let p = &edge.points;
                near(p[0], last) || near(p[p.len() - 1], last)
            }) else {
                break;
            };
            let next = open.remove(next);
            if next.footprint != footprint {
                footprint = None;
            }
            let mut points = next.points;
            if !near(points[0], last) {
                points.reverse();
            }
            chain.extend(points.into_iter().skip(1));
        }
    }
    loops
}

/// The points of a closed shape without repeats.
fn distinct(mut points: Vec<Point>) -> Vec<Point> {
    points.dedup();
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    const MM: i32 = 1_000_000;

    /// A 2-layer board 10 mm square with `items` on it, as KiCad 10 writes it.
    fn board(items: &str) -> Design {
        let text = format!(
            r#"(kicad_pcb (version 20260206)
                (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
                (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts"))
                {items})"#
        );
        read(&text, None, None).expect("valid board").design
    }

    fn net_names(design: &Design) -> Vec<&str> {
        design.nets.iter().map(|n| n.name.as_str()).collect()
    }

    #[test]
    fn kicad_9_refers_to_nets_by_number() {
        let design = board(
            r#"(net 0 "") (net 1 "GND") (net 2 "VCC")
               (footprint "R" (at 5 5) (property "Reference" "R1")
                 (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 2 "VCC")))
               (segment (start 1 1) (end 2 1) (width 0.2) (layer "F.Cu") (net 1))"#,
        );
        assert_eq!(net_names(&design), ["GND", "VCC"]);
        assert_eq!(design.pads[0].net, Some(NetId::from_index(1)));
        assert_eq!(design.existing.tracks[0].net, NetId::from_index(0));
    }

    #[test]
    fn pad_names_are_unique_when_a_footprint_repeats_a_number() {
        let design = board(
            r#"(footprint "U" (at 5 5) (property "Reference" "U1")
                 (pad "9" smd rect (at -1 0) (size 1 1) (layers "F.Cu"))
                 (pad "9" smd rect (at 1 0) (size 1 1) (layers "F.Cu")))"#,
        );
        let names: Vec<&str> = design.pads.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["U1-9", "U1-9@1"]);
    }

    #[test]
    fn a_hole_without_copper_keeps_everything_out_by_the_hole_clearance() {
        let design = board(
            r#"(footprint "H" (at 5 5) (property "Reference" "H1")
                 (pad "" np_thru_hole circle (at 1 0) (size 2 2) (drill 2) (layers "*.Cu" "*.Mask")))"#,
        );
        assert!(design.pads.is_empty());
        assert_eq!(
            design.keepouts,
            [Keepout {
                layer: None,
                // KiCad's default hole clearance, 0.25 mm, all around.
                shape: Shape::circle(Point::new(6 * MM, 5 * MM), 5 * MM / 2),
                scope: KeepoutScope::Everything,
                clearance: 0,
            }]
        );
    }

    #[test]
    fn pad_shapes_cover_kicads() {
        let design = board(
            r#"(footprint "X" (at 5 5) (property "Reference" "X1")
                 (pad "1" smd oval (at 0 0) (size 2 1) (layers "F.Cu"))
                 (pad "2" smd roundrect (at 0 3) (size 2 1) (layers "F.Cu") (roundrect_rratio 0.25))
                 (pad "3" thru_hole circle (at 3 0) (size 1.6 1.6) (drill 0.8 (offset 0.1 0)) (layers "*.Cu")))"#,
        );
        let shape = |i: usize| &design.pads[i].copper[0].shape;
        assert_eq!(
            *shape(0),
            Shape::track(
                Segment::new(
                    Point::new(9 * MM / 2, 5 * MM),
                    Point::new(11 * MM / 2, 5 * MM)
                ),
                MM
            )
        );
        let Shape::Polygon(rounded) = shape(1) else {
            panic!("a rounded rectangle is a polygon")
        };
        assert_eq!(rounded.points().len(), 8);
        assert_eq!(
            rounded.bounds(),
            Rect {
                min: Point::new(4 * MM, 15 * MM / 2),
                max: Point::new(6 * MM, 17 * MM / 2)
            }
        );
        assert_eq!(design.pads[2].copper.len(), 2, "on both layers");
        assert_eq!(
            *shape(2),
            Shape::circle(Point::new(8 * MM + MM / 10, 5 * MM), 16 * MM / 10)
        );
    }

    #[test]
    fn keepout_zones_keep_out_what_they_forbid() {
        let design = board(
            r#"(zone (net "") (layer "B.Cu")
                 (keepout (tracks not_allowed) (vias allowed) (pads allowed) (copperpour not_allowed) (footprints allowed))
                 (polygon (pts (xy 1 1) (xy 3 1) (xy 3 3) (xy 1 3))))
               (zone (net "") (layers "F.Cu" "B.Cu")
                 (keepout (tracks allowed) (vias allowed) (pads allowed) (copperpour not_allowed) (footprints allowed))
                 (polygon (pts (xy 5 5) (xy 6 5) (xy 6 6))))"#,
        );
        assert_eq!(
            design.keepouts.len(),
            1,
            "the zone that forbids no copper is no keepout"
        );
        assert_eq!(design.keepouts[0].layer, Some(LayerId::from_index(1)));
        assert_eq!(design.keepouts[0].scope, KeepoutScope::TracksOnly);
    }

    fn read_board(items: &str) -> Board {
        let text = format!(
            r#"(kicad_pcb (version 20260206)
                (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
                (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts"))
                {items})"#
        );
        read(&text, None, None).expect("valid board")
    }

    #[test]
    fn tracks_arcs_and_vias_the_designer_locked_are_kept_apart() {
        let design = read_board(
            r#"(net 1 "GND")
               (segment (start 1 1) (end 2 1) (width 0.2) (layer "F.Cu") (locked yes) (net 1))
               (segment locked (start 1 2) (end 2 2) (width 0.2) (layer "F.Cu") (net 1))
               (segment (start 1 3) (end 2 3) (width 0.2) (layer "F.Cu") (net 1))
               (via (at 3 3) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (locked yes) (net 1))
               (via (at 4 4) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 1))"#,
        )
        .design;
        assert_eq!(design.existing.tracks.len(), 3);
        assert_eq!(design.existing.vias.len(), 2);
        let ys: Vec<i32> = design.locked.tracks.iter().map(|t| t.path[0].y).collect();
        assert_eq!(ys, [1_000_000, 2_000_000]);
        assert_eq!(design.locked.vias.len(), 1);
        assert_eq!(design.locked.vias[0].position.x, 3_000_000);
    }

    #[test]
    fn knows_whether_the_board_has_copper_zones_to_fill() {
        let rule_area = r#"(zone (net "") (layer "B.Cu")
              (keepout (tracks not_allowed) (vias allowed) (pads allowed) (copperpour not_allowed) (footprints allowed))
              (polygon (pts (xy 1 1) (xy 3 1) (xy 3 3))))"#;
        let copper = r#"(zone (net "GND") (layer "B.Cu")
              (polygon (pts (xy 1 1) (xy 9 1) (xy 9 9))))"#;
        let in_a_footprint = format!(r#"(footprint "Logo" (at 5 5) {copper})"#);
        assert!(
            !read_board(rule_area).has_copper_zones,
            "a rule area is no copper"
        );
        assert!(read_board(copper).has_copper_zones);
        assert!(read_board(&in_a_footprint).has_copper_zones);
    }

    #[test]
    fn copper_zones_are_planes_of_their_net_on_each_of_their_layers() {
        let design = board(
            r#"(footprint "J" (at 5 5) (property "Reference" "J1")
                 (pad "1" thru_hole circle (at 0 0) (size 1.6 1.6) (drill 0.8) (layers "*.Cu") (net "GND")))
               (zone (net "GND") (net_name "GND") (layers "F.Cu" "B.Cu")
                 (polygon (pts (xy 1 1) (xy 9 1) (xy 9 9) (xy 1 9))))"#,
        );
        let gnd = NetId::from_index(0);
        let on: Vec<(NetId, LayerId)> = design.planes.iter().map(|p| (p.net, p.layer)).collect();
        assert_eq!(on, [(gnd, LayerId(0)), (gnd, LayerId(1))]);
        assert_eq!(
            design.planes[0].outline.bounds(),
            Rect {
                min: Point::new(MM, MM),
                max: Point::new(9 * MM, 9 * MM)
            }
        );
    }

    #[test]
    fn a_planes_fill_keeps_its_zones_clearance_and_minimum_width() {
        let design = board(
            r#"(footprint "J" (at 5 5) (property "Reference" "J1")
                 (pad "1" thru_hole circle (at 0 0) (size 1.6 1.6) (drill 0.8) (layers "*.Cu") (net "GND")))
               (zone (net "GND") (net_name "GND") (layer "B.Cu")
                 (connect_pads thru_hole_only (clearance 0.5)) (min_thickness 0.2)
                 (polygon (pts (xy 1 1) (xy 9 1) (xy 9 9) (xy 1 9))))"#,
        );
        let plane = &design.planes[0];
        assert_eq!((plane.clearance, plane.min_width), (MM / 2, MM / 5));
    }

    #[test]
    fn a_zone_of_a_net_no_pad_is_in_is_no_plane() {
        let design = board(
            r#"(zone (net "GND") (net_name "GND") (layer "B.Cu")
                 (polygon (pts (xy 1 1) (xy 9 1) (xy 9 9) (xy 1 9))))"#,
        );
        assert!(design.planes.is_empty());
        assert!(design.nets.is_empty(), "{:?}", design.nets);
    }

    /// A 4-layer board whose inner layers are of type power, with `items`.
    fn four_layers_of_power_inside(items: &str) -> Design {
        let text = format!(
            r#"(kicad_pcb (version 20260206)
                (layers (0 "F.Cu" signal) (4 "In1.Cu" power) (6 "In2.Cu" power) (2 "B.Cu" signal)
                        (25 "Edge.Cuts" user))
                (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts"))
                {items})"#
        );
        read(&text, None, None).expect("valid board").design
    }

    #[test]
    fn a_power_layer_holding_a_copper_zone_takes_no_tracks_and_one_without_does() {
        let design = four_layers_of_power_inside(
            r#"(zone (net "GND") (layer "In1.Cu")
                 (polygon (pts (xy 1 1) (xy 9 1) (xy 9 9) (xy 1 9))))"#,
        );
        let routable: Vec<bool> = design.layers.iter().map(|l| l.routable).collect();
        assert_eq!(routable, [true, false, true, true]);
    }

    #[test]
    fn a_loop_inside_the_outline_is_a_hole_in_the_board() {
        // A hole is board edge, which copper keeps the edge clearance from, not
        // a keepout, which it only may not touch.
        let design = board(r#"(gr_circle (center 5 5) (end 6 5) (layer "Edge.Cuts"))"#);
        assert_eq!(design.outline.polygons().len(), 1);
        assert_eq!(design.keepouts.len(), 0);
        assert_eq!(design.outline.holes().len(), 1);
        let hole = design.outline.holes()[0].bounds();
        assert!(hole.min.x >= 4 * MM && hole.max.x <= 6 * MM, "{hole:?}");
        assert!(hole.max.x > 6 * MM - 10_000);
        assert!(!design.outline.contains(Point::new(5 * MM, 5 * MM)));
    }

    #[test]
    fn a_cutout_a_footprint_draws_belongs_to_that_footprint() {
        // Custom rules may name it with memberOfFootprint, as TokenBoard's LED
        // cutouts are; a cutout of the board's own belongs to none.
        let design = board(
            r#"(gr_circle (center 2 2) (end 3 2) (layer "Edge.Cuts"))
               (footprint "SK6812" (at 7 7) (property "Reference" "LED1")
                 (fp_line (start -1 -1) (end 1 -1) (layer "Edge.Cuts"))
                 (fp_line (start 1 -1) (end 1 1) (layer "Edge.Cuts"))
                 (fp_line (start 1 1) (end -1 1) (layer "Edge.Cuts"))
                 (fp_line (start -1 1) (end -1 -1) (layer "Edge.Cuts")))"#,
        );
        let owners: Vec<(bool, Option<&str>)> = (design.outline.holes().iter())
            .zip(&design.cutout_footprints)
            .map(|(hole, owner)| (hole.bounds().min.x > 5 * MM, owner.as_deref()))
            .collect();
        assert_eq!(owners.len(), 2, "{owners:?}");
        assert!(owners.contains(&(false, None)), "{owners:?}");
        assert!(owners.contains(&(true, Some("LED1"))), "{owners:?}");
    }

    #[test]
    fn edges_join_end_to_end_in_any_direction_and_arcs_bend() {
        let text = r#"(kicad_pcb (version 20260206)
            (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
            (gr_line (start 0 0) (end 10 0) (layer "Edge.Cuts"))
            (gr_line (start 0 10) (end 0 0) (layer "Edge.Cuts"))
            (gr_arc (start 10 0) (mid 15 5) (end 10 10) (layer "Edge.Cuts"))
            (gr_line (start 0 10) (end 10 10) (layer "Edge.Cuts")))"#;
        let design = read(text, None, None).expect("valid").design;
        let bounds = design.outline.bounds();
        assert_eq!(bounds.min, Point::new(0, 0));
        assert_eq!(bounds.max, Point::new(15 * MM, 10 * MM));
        assert!(design.outline.polygons()[0].points().len() > 10);
    }

    #[test]
    fn an_arc_stays_within_its_tolerance_of_the_circle() {
        let (centre, radius) = (Point::new(0, 0), 50 * MM);
        let points = arc(
            Point::new(radius, 0),
            Point::new(0, radius),
            Point::new(-radius, 0),
        );
        for pair in points.windows(2) {
            let middle = Point::new((pair[0].x + pair[1].x) / 2, (pair[0].y + pair[1].y) / 2);
            let bulge = i64::from(radius) - centre.distance(middle);
            assert!(bulge <= ARC_TOLERANCE as i64 + 1, "{bulge}");
            assert!(pair[0].y >= 0, "the arc runs through its middle point");
        }
    }

    #[test]
    fn copper_of_no_net_keeps_other_copper_away() {
        let design = board(
            r#"(segment (start 1 1) (end 2 1) (width 0.2) (layer "F.Cu") (net ""))
               (via (at 3 3) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu"))"#,
        );
        assert!(design.existing.tracks.is_empty() && design.existing.vias.is_empty());
        assert_eq!(design.keepouts.len(), 2);
    }

    #[test]
    fn a_board_without_a_closed_outline_is_refused() {
        let text = r#"(kicad_pcb (layers (0 "F.Cu" signal))
            (gr_line (start 0 0) (end 10 0) (layer "Edge.Cuts")))"#;
        assert!(read(text, None, None).is_err());
        assert!(read("(kicad_sch)", None, None).is_err());
    }
}
