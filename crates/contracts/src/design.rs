use crate::{LayerId, NetClassId, NetId, PadId, Routes, ViaKindId};
use traxide_geometry::{Point, Polygon, Region, Shape};

/// A board to route, independent of the format it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Design {
    /// Copper layers from top to bottom.
    pub layers: Vec<Layer>,
    /// The board's outline: one polygon, or one per board of a panel.
    pub outline: Region,
    /// The footprint that draws each hole of the outline, by reference, in the
    /// order of its holes; `None` where the board draws it.
    pub cutout_footprints: Vec<Option<String>>,
    pub nets: Vec<Net>,
    pub pads: Vec<Pad>,
    pub via_kinds: Vec<ViaKind>,
    pub keepouts: Vec<Keepout>,
    /// Copper pours of a net, which the board's program fills around the
    /// copper of other nets.
    pub planes: Vec<Plane>,
    /// Copper already on the board; routing keeps it as it is.
    pub existing: Routes,
    /// The part of it the board's program keeps itself, as it is locked there:
    /// a session leaves it out, as KiCad's import keeps it and would add it twice.
    pub locked: Routes,
    pub rules: DesignRules,
    /// The coordinate step of the board's format. Routes are rounded to it when
    /// stored, so routing keeps that much extra clearance.
    pub resolution: i32,
    /// How far the outline as read may lie inside the board's true edges, as
    /// where it reads an arc as chords: copper keeps that much more clearance
    /// from the edges.
    pub outline_tolerance: i32,
    /// What the reader found wrong with the board and read anyway, for the
    /// designer.
    pub warnings: Vec<String>,
}

impl Design {
    pub fn net(&self, id: NetId) -> &Net {
        &self.nets[id.index()]
    }

    pub fn pad(&self, id: PadId) -> &Pad {
        &self.pads[id.index()]
    }

    pub fn via_kind(&self, id: ViaKindId) -> &ViaKind {
        &self.via_kinds[id.index()]
    }

    pub fn layer_ids(&self) -> impl Iterator<Item = LayerId> + use<> {
        (0..self.layers.len()).map(LayerId::from_index)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    pub name: String,
    /// Plane layers carry copper but no tracks.
    pub routable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Net {
    pub name: String,
    pub class: NetClassId,
    pub pads: Vec<PadId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pad {
    /// Unique on the board, such as `R1-2`.
    pub name: String,
    pub net: Option<NetId>,
    pub position: Point,
    /// Absolute shapes per layer; a through-hole pad has one on every layer.
    pub copper: Vec<LayerShape>,
}

impl Pad {
    /// The part the pad belongs to: its name up to the first `-`, such as `R1`
    /// of `R1-2`; a pin's own name may hold one, as `-D+` of `J3--D+`.
    pub fn part(&self) -> &str {
        self.name
            .split_once('-')
            .map_or(self.name.as_str(), |(part, _)| part)
    }

    pub fn is_on(&self, layer: LayerId) -> bool {
        self.copper.iter().any(|c| c.layer == layer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerShape {
    pub layer: LayerId,
    pub shape: Shape,
}

/// A via padstack. Its copper is relative to the via's centre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViaKind {
    pub name: String,
    pub copper: Vec<LayerShape>,
    /// The diameter of its hole, where the board's format states it.
    pub drill: Option<i32>,
}

impl ViaKind {
    pub fn connects(&self, from: LayerId, to: LayerId) -> bool {
        let on = |layer| self.copper.iter().any(|c| c.layer == layer);
        on(from) && on(to)
    }
}

/// A pour of one net on one layer: the board's program fills `outline` with
/// copper of `net`, less a clearance around the copper of other nets, and the
/// fill joins the copper of `net` it reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plane {
    pub net: NetId,
    pub layer: LayerId,
    pub outline: Polygon,
    /// Cut out of it, as windows: nothing is poured there.
    pub holes: Vec<Polygon>,
    /// How far its fill keeps from other nets' copper where its own setting
    /// asks more than the rules; 0 where the format keeps none.
    pub clearance: i32,
    /// How narrow its fill may get: narrower necks are left out of it.
    pub min_width: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keepout {
    /// `None` blocks every layer.
    pub layer: Option<LayerId>,
    pub shape: Shape,
    pub scope: KeepoutScope,
    /// How far copper keeps from it: a Specctra keepout asks the clearance as
    /// copper does, a rule area of KiCad's only that copper stays out.
    pub clearance: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepoutScope {
    Everything,
    TracksOnly,
    ViasOnly,
}

/// The rules as the board states them. Only the Rules Engine interprets them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignRules {
    pub classes: Vec<NetClass>,
    /// The class of copper that belongs to no net, such as a mounting hole's pad.
    pub default_class: NetClassId,
    pub edge_clearance: i32,
    /// How far copper of other nets stays from a via's hole.
    pub hole_clearance: i32,
    /// How far apart the holes of vias stay, whatever their nets.
    pub hole_to_hole: i32,
    /// The narrowest a track may be, as the board's own settings state it; 0
    /// where the format states none, as Specctra, whose class widths are the
    /// widths to route at rather than limits.
    pub min_track_width: i32,
    /// The smallest a via's copper may be across, and its drill; 0 where the
    /// format states none.
    pub min_via_diameter: i32,
    pub min_via_drill: i32,
    /// Rules for particular copper beyond those of its class, in the order the
    /// board states them: where several apply, the last one counts, and it
    /// overrides the class's and the board's values.
    pub custom: Vec<CustomRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetClass {
    pub name: String,
    pub track_width: i32,
    pub clearance: i32,
    /// In order of preference.
    pub via_kinds: Vec<ViaKindId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomRule {
    pub name: String,
    pub condition: Condition,
    pub constraint: Constraint,
    pub limits: Limits,
}

/// What a custom rule constrains. Between two items (clearances) the rule's
/// condition is tried both ways round; for one item, its item is [`Item::A`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constraint {
    /// Between copper of two nets.
    Clearance,
    TrackWidth,
    ViaDiameter,
    /// The drill of a via.
    HoleSize,
    /// Between copper and the board's edge.
    EdgeClearance,
    /// Between a via's hole and copper of another net.
    HoleClearance,
}

/// Nanometres; `None` where the rule leaves a limit out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Limits {
    pub min: Option<i32>,
    pub opt: Option<i32>,
    pub max: Option<i32>,
}

/// When a custom rule applies, as far as the board's reader could tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    Always,
    /// The constrained copper lies on one of these layers.
    OnLayers(Vec<LayerId>),
    NetClassIn {
        item: Item,
        classes: Vec<NetClassId>,
    },
    /// `None` stands for copper of no net.
    NetIn {
        item: Item,
        nets: Vec<Option<NetId>>,
    },
    /// The item belongs to one of these footprints, by reference. Routed copper
    /// belongs to none; a cutout to the footprint that draws it.
    MemberOfFootprint {
        item: Item,
        footprints: Vec<String>,
    },
    Not(Box<Condition>),
    All(Vec<Condition>),
    Any(Vec<Condition>),
    /// A test the reader cannot express, which may hold or not.
    Unknown,
}

/// The two items a rule compares: the constrained one and, for clearances, the
/// other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    A,
    B,
}
