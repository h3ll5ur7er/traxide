use super::sexpr::{self, Sexp};
use super::shape::{self, LayeredShape, layered_shape};
use super::units::Units;
use crate::BoardError;
use std::collections::HashMap;
use traxide_contracts::geometry::{Point, Polygon, Rect, Region, Shape, Transform};
use traxide_contracts::{
    Design, DesignRules, Keepout, KeepoutScope, Layer, LayerId, LayerShape, Net, NetClass,
    NetClassId, NetId, Pad, PadId, Plane, Routes, Track, Via, ViaKind, ViaKindId,
};

pub fn parse_dsn(text: &str) -> Result<Design, BoardError> {
    let not_a_design = |line| BoardError::Invalid {
        line,
        message: "not a Specctra design: expected (pcb …)".into(),
    };
    // Other programs save other things as .dsn too, such as OrCAD schematics.
    let text = text.trim_start_matches('\u{feff}');
    if !text.trim_start().starts_with('(') {
        return Err(not_a_design(1));
    }
    let root = sexpr::parse(text)?;
    if root.keyword() != Some("pcb") {
        return Err(not_a_design(root.line()));
    }
    DsnReader::new(&root)?.read()
}

/// KiCad 7.0.2 wrote the keepout around a footprint's hole 500 mm too wide, so
/// that it covered the whole board (fixed by 7.0.4). No footprint has a hole
/// half a metre wide: a keepout circle that wide gets its size back.
fn without_kicad_7_0_2_excess(shape: Shape) -> Shape {
    const EXCESS_RADIUS: i32 = 250_000_000;
    match shape {
        Shape::Capsule { segment, radius } if segment.a == segment.b && radius > EXCESS_RADIUS => {
            Shape::Capsule {
                segment,
                radius: radius - EXCESS_RADIUS,
            }
        }
        other => other,
    }
}

/// The corners of a polyline path, as Freerouting writes wires: its numbers are
/// lines, two points each, and each corner is where one line crosses the next.
fn corners(points: &[Point]) -> Vec<Point> {
    let lines: Vec<(Point, Point)> = points
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[a, b]| (a, b))
        .collect();
    lines
        .windows(2)
        .filter_map(|pair| crossing(pair[0], pair[1]))
        .collect()
}

/// Where two lines, each through two points, cross; `None` if they are parallel.
fn crossing((a, b): (Point, Point), (c, d): (Point, Point)) -> Option<Point> {
    let (ab, cd, ac) = (
        (i128::from(b.x - a.x), i128::from(b.y - a.y)),
        (i128::from(d.x - c.x), i128::from(d.y - c.y)),
        (i128::from(c.x - a.x), i128::from(c.y - a.y)),
    );
    let denominator = ab.0 * cd.1 - ab.1 * cd.0;
    if denominator == 0 {
        return None;
    }
    let numerator = ac.0 * cd.1 - ac.1 * cd.0;
    // a + ab · numerator / denominator, to the nearest nanometre.
    let along = |start: i32, step: i128| {
        i32::try_from(i128::from(start) + rounded_quotient(step * numerator, denominator)).ok()
    };
    Some(Point::new(along(a.x, ab.0)?, along(a.y, ab.1)?))
}

/// `numerator / denominator`, rounded half away from zero.
fn rounded_quotient(numerator: i128, denominator: i128) -> i128 {
    let (numerator, denominator) = if denominator < 0 {
        (-numerator, -denominator)
    } else {
        (numerator, denominator)
    };
    let magnitude = (2 * numerator.abs() + denominator) / (2 * denominator);
    magnitude * numerator.signum()
}

/// Specctra's y axis points up; the design's, like KiCad's, points down.
fn to_board(p: Point) -> Point {
    Point::new(p.x, -p.y)
}

struct Image<'a> {
    pins: Vec<Pin<'a>>,
    keepouts: Vec<(KeepoutScope, LayeredShape)>,
}

struct Pin<'a> {
    padstack: &'a str,
    id: &'a str,
    at: Point,
    rotation_degrees: f64,
}

struct DsnReader<'a> {
    root: &'a Sexp,
    structure: &'a Sexp,
    units: Units,
    resolution_nm: i32,
    layers: Vec<Layer>,
    layer_by_name: HashMap<&'a str, LayerId>,
}

impl<'a> DsnReader<'a> {
    fn new(root: &'a Sexp) -> Result<Self, BoardError> {
        let structure = root.required_child("structure")?;
        let planes: Vec<&str> = structure
            .children("plane")
            .flat_map(Sexp::args)
            .filter(|shape| shape.keyword().is_some())
            .filter_map(|shape| shape.args().first().and_then(Sexp::atom))
            .collect();
        let from_kicad = is_from_kicad(root);
        let mut layers = Vec::new();
        let mut layer_by_name = HashMap::new();
        for layer in structure.children("layer") {
            let name = layer.arg_atom(0, "layer name")?;
            let kind = layer.child("type").and_then(|t| t.atoms().next());
            // KiCad writes `power` for a copper layer of that type, which it
            // routes like any other unless a plane fills it.
            let is_plane = kind == Some("power") && (!from_kicad || planes.contains(&name));
            layer_by_name.insert(name, LayerId::from_index(layers.len()));
            layers.push(Layer {
                name: name.to_owned(),
                routable: !is_plane,
            });
        }
        if layers.is_empty() {
            return Err(structure.invalid("the design has no layers"));
        }
        let units = Units::of_design(root)?;
        Ok(Self {
            root,
            structure,
            units: units.design_lengths(),
            resolution_nm: units.resolution_nm(),
            layers,
            layer_by_name,
        })
    }

    fn read(self) -> Result<Design, BoardError> {
        let padstacks = self.padstacks()?;
        let (mut pads, footprint_keepouts) = self.placed(&padstacks)?;
        let pad_by_name: HashMap<String, PadId> = pads
            .iter()
            .enumerate()
            .map(|(i, p)| (p.name.clone(), PadId::from_index(i)))
            .collect();
        let mut via_kinds = ViaKinds {
            kinds: Vec::new(),
            from_kicad: self.is_from_kicad(),
        };
        let rules = self.rules(&padstacks, &mut via_kinds)?;
        let mut nets = self.nets(&pad_by_name)?;
        let mut warnings = Vec::new();
        for index in 0..nets.len() {
            nets[index].class = rules
                .class_of_net
                .get(nets[index].name.as_str())
                .copied()
                .unwrap_or(rules.design.default_class);
            for pad in nets[index].pads.clone() {
                if let Some(earlier) = pads[pad.index()].net
                    && earlier.index() != index
                {
                    warnings.push(format!(
                        "pin {} is listed in the nets {} and {}; it stays in {}",
                        pads[pad.index()].name,
                        nets[earlier.index()].name,
                        nets[index].name,
                        nets[index].name
                    ));
                    nets[earlier.index()].pads.retain(|&p| p != pad);
                }
                pads[pad.index()].net = Some(NetId::from_index(index));
            }
        }
        let (existing, locked) = self.wiring(&nets, &padstacks, &mut via_kinds)?;
        // Freerouting keeps the clearance from keepouts, as from copper.
        let clearance = rules.design.classes[rules.design.default_class.index()].clearance;
        let keepouts = [self.keepouts()?, footprint_keepouts]
            .concat()
            .into_iter()
            .map(|keepout| Keepout {
                clearance,
                ..keepout
            })
            .collect();
        Ok(Design {
            outline: self.outline()?,
            cutout_footprints: Vec::new(),
            locked,
            keepouts,
            planes: self.planes(&nets)?,
            layers: self.layers,
            nets,
            pads,
            via_kinds: via_kinds.kinds,
            existing,
            rules: rules.design,
            resolution: self.resolution_nm,
            outline_tolerance: 0,
            warnings,
        })
    }

    fn layers_named(&self, name: &Sexp) -> Result<Vec<LayerId>, BoardError> {
        let text = name
            .atom()
            .ok_or_else(|| name.invalid("expected a layer name"))?;
        match text {
            "signal" | "pcb" | "*" => Ok((0..self.layers.len()).map(LayerId::from_index).collect()),
            _ => self
                .layer_by_name
                .get(text)
                .map(|&id| vec![id])
                .ok_or_else(|| name.invalid(format!("unknown layer {text}"))),
        }
    }

    fn shape(&self, sexp: &Sexp) -> Result<LayeredShape, BoardError> {
        layered_shape(sexp, &self.units, |name| self.layers_named(name))
    }

    fn polygon(&self, sexp: &Sexp, coordinates: &[Sexp]) -> Result<Polygon, BoardError> {
        shape::polygon(sexp, &self.units, coordinates)
    }

    /// The signal boundaries, which routing must stay inside, where the design
    /// has them; otherwise the board's outline (the `pcb` boundaries). A panel
    /// of several boards has one per board.
    fn outline(&self) -> Result<Region, BoardError> {
        let first = self.structure.required_child("boundary")?;
        let shapes: Vec<&Sexp> = self
            .structure
            .children("boundary")
            .flat_map(Sexp::args)
            .filter(|shape| shape.keyword().is_some_and(|k| k != "window"))
            .collect();
        let mut holes = Vec::new();
        for boundary in self.structure.children("boundary") {
            holes.extend(self.windows(boundary)?);
        }
        let is_signal = |shape: &&Sexp| shape.args().first().and_then(Sexp::atom) == Some("signal");
        let chosen: Vec<&Sexp> = if shapes.iter().any(is_signal) {
            shapes.into_iter().filter(is_signal).collect()
        } else {
            shapes
        };
        let polygons = chosen
            .into_iter()
            .map(|shape| self.outline_polygon(shape))
            .collect::<Result<Vec<_>, BoardError>>()?;
        Region::new(polygons)
            .map(|region| region.with_holes(holes))
            .ok_or_else(|| first.invalid("empty boundary"))
    }

    /// A polygon of the boundary or a window, in board coordinates.
    fn outline_polygon(&self, shape: &Sexp) -> Result<Polygon, BoardError> {
        let polygon = match shape.keyword() {
            Some("path" | "polygon") => {
                self.polygon(shape, shape.args().get(2..).unwrap_or_default())?
            }
            Some("rect") => {
                let corners = self.units.points(&shape.args()[1..])?;
                Polygon::rectangle(
                    Rect::around(corners).ok_or_else(|| shape.invalid("empty rect"))?,
                )
            }
            _ => return Err(shape.invalid("unsupported outline shape")),
        };
        Ok(polygon.map(to_board))
    }

    /// The windows of `parent`: holes cut out of it.
    fn windows(&self, parent: &Sexp) -> Result<Vec<Polygon>, BoardError> {
        let mut holes = Vec::new();
        for window in parent.children("window") {
            for shape in window.args().iter().filter(|a| a.keyword().is_some()) {
                holes.push(self.outline_polygon(shape)?);
            }
        }
        Ok(holes)
    }

    /// `(plane NET (polygon LAYER …) (window …)…)`, as KiCad writes its copper
    /// zones, a window for each hole. A plane of a net the network does not
    /// list joins no pin, and is left out.
    fn planes(&self, nets: &[Net]) -> Result<Vec<Plane>, BoardError> {
        let mut planes = Vec::new();
        for plane in self.structure.children("plane") {
            let name = plane.arg_atom(0, "net name")?;
            let Some(net) = nets.iter().position(|n| n.name == name) else {
                continue;
            };
            let holes = self.windows(plane)?;
            let shapes =
                (plane.args().iter()).filter(|a| a.keyword().is_some_and(|k| k != "window"));
            for shape in shapes {
                let LayeredShape { layers, shape } = self.shape(shape)?;
                let Shape::Polygon(outline) = shape else {
                    continue;
                };
                planes.extend(layers.into_iter().map(|layer| Plane {
                    net: NetId::from_index(net),
                    layer,
                    outline: outline.map(to_board),
                    holes: holes.clone(),
                    clearance: 0,
                    min_width: 0,
                }));
            }
        }
        Ok(planes)
    }

    fn keepouts(&self) -> Result<Vec<Keepout>, BoardError> {
        let keepouts = self.keepouts_in(self.structure)?;
        Ok(keepouts
            .into_iter()
            .flat_map(|(scope, LayeredShape { layers, shape })| {
                self.keepouts_on(&layers, shape.map(to_board), scope)
            })
            .collect())
    }

    /// The keepouts listed in `parent`, the structure or an image, as Specctra states them.
    fn keepouts_in(&self, parent: &Sexp) -> Result<Vec<(KeepoutScope, LayeredShape)>, BoardError> {
        let scopes = [
            ("keepout", KeepoutScope::Everything),
            ("wire_keepout", KeepoutScope::TracksOnly),
            ("via_keepout", KeepoutScope::ViasOnly),
        ];
        let mut keepouts = Vec::new();
        for (keyword, scope) in scopes {
            for keepout in parent.children(keyword) {
                let shape = keepout
                    .args()
                    .iter()
                    .find(|a| matches!(a.keyword(), Some("circle" | "rect" | "polygon" | "path")))
                    .ok_or_else(|| keepout.invalid("keepout without a shape"))?;
                let mut layered = self.shape(shape)?;
                if self.is_from_kicad() {
                    layered.shape = without_kicad_7_0_2_excess(layered.shape);
                }
                keepouts.push((scope, layered));
            }
        }
        Ok(keepouts)
    }

    /// One keepout for all layers, or one per layer it covers.
    fn keepouts_on(&self, layers: &[LayerId], shape: Shape, scope: KeepoutScope) -> Vec<Keepout> {
        if layers.len() == self.layers.len() {
            return vec![Keepout {
                layer: None,
                shape,
                scope,
                clearance: 0,
            }];
        }
        layers
            .iter()
            .map(|&layer| Keepout {
                layer: Some(layer),
                shape: shape.clone(),
                scope,
                clearance: 0,
            })
            .collect()
    }

    fn padstacks(&self) -> Result<HashMap<&'a str, Vec<LayeredShape>>, BoardError> {
        let library = self.root.required_child("library")?;
        library
            .children("padstack")
            .map(|padstack| {
                let name = padstack.arg_atom(0, "padstack name")?;
                let shapes = padstack
                    .children("shape")
                    .filter_map(|s| s.args().first())
                    .map(|s| self.shape(s))
                    .collect::<Result<_, _>>()?;
                Ok((name, shapes))
            })
            .collect()
    }

    fn images(&self) -> Result<HashMap<&'a str, Image<'a>>, BoardError> {
        let library = self.root.required_child("library")?;
        library
            .children("image")
            .map(|image| {
                let name = image.arg_atom(0, "image name")?;
                let pins = image
                    .children("pin")
                    .map(|pin| self.pin(pin))
                    .collect::<Result<_, _>>()?;
                Ok((
                    name,
                    Image {
                        pins,
                        keepouts: self.keepouts_in(image)?,
                    },
                ))
            })
            .collect()
    }

    /// `(pin padstack [(rotate degrees)] id x y)`
    fn pin(&self, pin: &'a Sexp) -> Result<Pin<'a>, BoardError> {
        let atoms: Vec<&Sexp> = pin.args().iter().filter(|a| a.atom().is_some()).collect();
        let [padstack, id, x, y] = atoms[..] else {
            return Err(pin.invalid("expected (pin padstack id x y)"));
        };
        let rotation_degrees = match pin.child("rotate") {
            Some(rotate) => degrees(rotate, rotate.arg_atom(0, "angle")?)?,
            None => 0.0,
        };
        Ok(Pin {
            padstack: padstack.atom().unwrap_or_default(),
            id: id.atom().unwrap_or_default(),
            at: self.units.point(x, y)?,
            rotation_degrees,
        })
    }

    /// The pads and keepouts of every placed component.
    fn placed(
        &self,
        padstacks: &HashMap<&str, Vec<LayeredShape>>,
    ) -> Result<(Vec<Pad>, Vec<Keepout>), BoardError> {
        let images = self.images()?;
        let last_layer = self.layers.len() - 1;
        let mut pads = Vec::new();
        let mut keepouts = Vec::new();
        for component in self.root.required_child("placement")?.children("component") {
            let image_name = component.arg_atom(0, "image name")?;
            let image = images
                .get(image_name)
                .ok_or_else(|| component.invalid(format!("unknown image {image_name}")))?;
            for place in component.children("place") {
                let reference = place.arg_atom(0, "reference")?;
                let args = place.args();
                let (Some(x), Some(y)) = (args.get(1), args.get(2)) else {
                    return Err(place.invalid("place without a position"));
                };
                let back = place.arg_atom(3, "side").is_ok_and(|side| side == "back");
                let rotation_degrees = match args.get(4).and_then(Sexp::atom) {
                    Some(angle) => degrees(place, angle)?,
                    None => 0.0,
                };
                let placement = Transform {
                    origin: self.units.point(x, y)?,
                    rotation_degrees,
                    mirror_x: back,
                };
                // Placing a component on the back mirrors its layer stack too.
                let layer_on_side = |l: LayerId| {
                    if back {
                        LayerId::from_index(last_layer - l.index())
                    } else {
                        l
                    }
                };
                for (scope, LayeredShape { layers, shape }) in &image.keepouts {
                    let layers: Vec<LayerId> = layers.iter().map(|&l| layer_on_side(l)).collect();
                    let shape = shape.map(|p| to_board(placement.apply(p)));
                    keepouts.extend(self.keepouts_on(&layers, shape, *scope));
                }
                for pin in &image.pins {
                    let shapes = padstacks.get(pin.padstack).ok_or_else(|| {
                        place.invalid(format!("unknown padstack {}", pin.padstack))
                    })?;
                    let local = Transform {
                        origin: pin.at,
                        rotation_degrees: pin.rotation_degrees,
                        mirror_x: false,
                    };
                    let to_absolute = local.then(placement);
                    let copper = shapes
                        .iter()
                        .flat_map(|s| s.layers.iter().map(move |&l| (l, &s.shape)))
                        .map(|(layer, shape)| LayerShape {
                            layer: layer_on_side(layer),
                            shape: shape.map(|p| to_board(to_absolute(p))),
                        })
                        .collect();
                    pads.push(Pad {
                        name: format!("{reference}-{}", pin.id),
                        net: None,
                        position: to_board(placement.apply(pin.at)),
                        copper,
                    });
                }
            }
        }
        Ok((pads, keepouts))
    }

    fn rules(
        &self,
        padstacks: &HashMap<&str, Vec<LayeredShape>>,
        via_kinds: &mut ViaKinds,
    ) -> Result<CompiledClasses<'a>, BoardError> {
        let board_rules: Vec<&Sexp> = self.structure.children("rule").collect();
        let (width, clearance) = self.rule(self.structure, &board_rules, None)?;
        let mut default_vias = Vec::new();
        for via in self.structure.children("via") {
            for name in via.args() {
                default_vias.push(via_kinds.named(name, padstacks)?);
            }
        }
        let mut classes = vec![NetClass {
            name: "default".into(),
            track_width: width,
            clearance,
            via_kinds: default_vias.clone(),
        }];
        let mut class_of_net = HashMap::new();
        for class in self.root.required_child("network")?.children("class") {
            let id = NetClassId::from_index(classes.len());
            // Some tools write a class without a name, which no net can be in.
            let Some(name) = class.args().first().and_then(Sexp::atom) else {
                continue;
            };
            let rules: Vec<&Sexp> = class.children("rule").collect();
            let (track_width, clearance) = self.rule(class, &rules, Some((width, clearance)))?;
            let mut vias = Vec::new();
            for use_via in class
                .children("circuit")
                .flat_map(|c| c.children("use_via"))
            {
                for name in use_via.args() {
                    vias.push(via_kinds.named(name, padstacks)?);
                }
            }
            for net in class.atoms().skip(1) {
                class_of_net.insert(net, id);
            }
            classes.push(NetClass {
                name: name.to_owned(),
                track_width,
                clearance,
                via_kinds: if vias.is_empty() {
                    default_vias.clone()
                } else {
                    vias
                },
            });
        }
        Ok(CompiledClasses {
            design: DesignRules {
                classes,
                default_class: NetClassId(0),
                edge_clearance: self.edge_clearance(clearance),
                hole_clearance: 0,
                // A DSN carries none: KiCad's default, which JLCPCB asks too.
                hole_to_hole: 250_000,
                min_track_width: 0,
                custom: vec![],
            },
            class_of_net,
        })
    }

    /// DSN cannot state how far copper must stay from the board edge. KiCad's DRC
    /// applies its board setup, which is 0.5 mm unless the designer changed it,
    /// so boards from KiCad keep at least that; others the board-wide clearance.
    fn edge_clearance(&self, clearance: i32) -> i32 {
        const KICAD_DEFAULT_EDGE_CLEARANCE: i32 = 500_000;
        if self.is_from_kicad() {
            clearance.max(KICAD_DEFAULT_EDGE_CLEARANCE)
        } else {
            clearance
        }
    }

    fn is_from_kicad(&self) -> bool {
        is_from_kicad(self.root)
    }

    /// The width and the general clearance that the `(rule …)` lists of `owner`
    /// set together, as some tools write one list per rule. Clearances limited to a `(type …)` of object refine pad-to-pad
    /// spacing, which routing never sets.
    fn rule(
        &self,
        owner: &Sexp,
        rules: &[&Sexp],
        defaults: Option<(i32, i32)>,
    ) -> Result<(i32, i32), BoardError> {
        let length = |keyword: &str, default: Option<i32>| -> Result<i32, BoardError> {
            let general = rules
                .iter()
                .flat_map(|rule| rule.children(keyword))
                .find(|item| item.child("type").is_none());
            match (general.and_then(|c| c.args().first()), default) {
                (Some(value), _) => self.units.length(value),
                (None, Some(default)) => Ok(default),
                (None, None) => Err(owner.invalid(format!("rules without a {keyword}"))),
            }
        };
        Ok((
            length("width", defaults.map(|d| d.0))?,
            length("clearance", defaults.map(|d| d.1))?,
        ))
    }

    fn nets(&self, pad_by_name: &HashMap<String, PadId>) -> Result<Vec<Net>, BoardError> {
        self.root
            .required_child("network")?
            .children("net")
            .map(|net| {
                let name = net.arg_atom(0, "net name")?;
                let pads = net
                    .children("pins")
                    .flat_map(|pins| pins.args())
                    .map(|pin| {
                        let pin_name = pin.atom().ok_or_else(|| pin.invalid("expected a pin"))?;
                        // A pin's name may be quoted after its component's:
                        // X14-"D-" for the pin declared as "D-".
                        let unquoted = pin_name.replace('"', "");
                        pad_by_name
                            .get(&unquoted)
                            .copied()
                            .ok_or_else(|| pin.invalid(format!("unknown pin {pin_name}")))
                    })
                    .collect::<Result<_, _>>()?;
                Ok(Net {
                    name: name.to_owned(),
                    class: NetClassId(0),
                    pads,
                })
            })
            .collect()
    }

    fn wiring(
        &self,
        nets: &[Net],
        padstacks: &HashMap<&str, Vec<LayeredShape>>,
        via_kinds: &mut ViaKinds,
    ) -> Result<(Routes, Routes), BoardError> {
        let (mut routes, mut locked) = (Routes::default(), Routes::default());
        let Some(wiring) = self.root.child("wiring") else {
            return Ok((routes, locked));
        };
        // KiCad exports locked copper as fixed; Freerouting writes the wiring
        // it was given back as protected.
        let is_locked = |item: &Sexp| {
            item.child("type")
                .and_then(|t| t.atoms().next())
                .is_some_and(|t| t == "fix" || t == "protect")
        };
        let net_of = |item: &Sexp| -> Result<NetId, BoardError> {
            let name = item.required_child("net")?.arg_atom(0, "net name")?;
            nets.iter()
                .position(|n| n.name == name)
                .map(NetId::from_index)
                .ok_or_else(|| item.invalid(format!("unknown net {name}")))
        };
        for wire in wiring.children("wire") {
            let (shape, points) = match (wire.child("path"), wire.child("polyline_path")) {
                (Some(path), _) => (
                    path,
                    self.units
                        .points(path.args().get(2..).unwrap_or_default())?,
                ),
                (None, Some(polyline)) => (
                    polyline,
                    corners(
                        &self
                            .units
                            .points(polyline.args().get(2..).unwrap_or_default())?,
                    ),
                ),
                (None, None) => return Err(wire.invalid("missing (path …)")),
            };
            let layer = self.layers_named(
                shape
                    .args()
                    .first()
                    .ok_or_else(|| shape.invalid("missing layer"))?,
            )?;
            let track = Track {
                net: net_of(wire)?,
                layer: layer[0],
                width: self.units.length(
                    shape
                        .args()
                        .get(1)
                        .ok_or_else(|| shape.invalid("missing width"))?,
                )?,
                path: points.into_iter().map(to_board).collect(),
            };
            if is_locked(wire) {
                locked.tracks.push(track.clone());
            }
            routes.tracks.push(track);
        }
        for via in wiring.children("via") {
            let args = via.args();
            let (Some(name), Some(x), Some(y)) = (args.first(), args.get(1), args.get(2)) else {
                return Err(via.invalid("expected (via padstack x y)"));
            };
            let placed = Via {
                net: net_of(via)?,
                kind: via_kinds.named(name, padstacks)?,
                position: to_board(self.units.point(x, y)?),
            };
            if is_locked(via) {
                locked.vias.push(placed.clone());
            }
            routes.vias.push(placed);
        }
        Ok((routes, locked))
    }
}

struct CompiledClasses<'a> {
    design: DesignRules,
    class_of_net: HashMap<&'a str, NetClassId>,
}

struct ViaKinds {
    kinds: Vec<ViaKind>,
    from_kicad: bool,
}

impl ViaKinds {
    fn named(
        &mut self,
        name: &Sexp,
        padstacks: &HashMap<&str, Vec<LayeredShape>>,
    ) -> Result<ViaKindId, BoardError> {
        let text = name
            .atom()
            .ok_or_else(|| name.invalid("expected a via padstack"))?;
        if let Some(index) = self.kinds.iter().position(|k| k.name == text) {
            return Ok(ViaKindId::from_index(index));
        }
        let shapes = padstacks
            .get(text)
            .ok_or_else(|| name.invalid(format!("unknown via padstack {text}")))?;
        let copper = shapes
            .iter()
            .flat_map(|s| {
                s.layers.iter().map(move |&layer| LayerShape {
                    layer,
                    shape: s.shape.map(to_board),
                })
            })
            .collect();
        self.kinds.push(ViaKind {
            name: text.to_owned(),
            copper,
            drill: self.from_kicad.then(|| kicad_via_drill(text)).flatten(),
        });
        Ok(ViaKindId::from_index(self.kinds.len() - 1))
    }
}

/// The drill of a via padstack KiCad named, as in `Via[0-3]_600:300_um`: its
/// diameter and drill in micrometres.
fn kicad_via_drill(name: &str) -> Option<i32> {
    let sizes = name.strip_suffix("_um")?.rsplit('_').next()?;
    let (_, drill) = sizes.split_once(':')?;
    let micrometres: f64 = drill.parse().ok()?;
    Some((micrometres * 1_000.0).round() as i32)
}

fn is_from_kicad(root: &Sexp) -> bool {
    root.child("parser")
        .and_then(|parser| parser.child("host_cad"))
        .and_then(|host| host.atoms().next())
        .is_some_and(|host| host.contains("KiCad"))
}

fn degrees(at: &Sexp, text: &str) -> Result<f64, BoardError> {
    text.parse()
        .map_err(|_| at.invalid(format!("not an angle: {text}")))
}
