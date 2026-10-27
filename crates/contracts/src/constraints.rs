use crate::{Limits, NetClassId, NetId, ViaKindId};

/// The rules compiled for routing: everything a router asks, answered in O(1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintSet {
    class_of_net: Vec<NetClassId>,
    default_class: NetClassId,
    track_width_of_class: Vec<i32>,
    via_kinds_of_class: Vec<Vec<ViaKindId>>,
    /// Row-major, classes × classes.
    clearance_between_classes: Vec<i32>,
    /// The largest of them, which every copper query asks: a board with
    /// hundreds of classes has a table of tens of thousands.
    largest_clearance: i32,
    edge_clearance: i32,
    /// Per hole of the outline, in its order, where rules on the footprint
    /// that draws it set another; the rest keep `edge_clearance`.
    cutout_clearances: Vec<i32>,
    hole_to_hole: i32,
    /// Per class, how narrow and how wide its tracks may be: limits to judge
    /// by, not the width to route at.
    track_width_limits_of_class: Vec<Limits>,
    /// Per class, how large across and how large a drill its vias may have.
    via_limits_of_class: Vec<ViaLimits>,
}

/// How large across, and how large a drill, a via may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViaLimits {
    pub diameter: Limits,
    pub drill: Limits,
}

impl ConstraintSet {
    /// Tables indexed by class; `clearance_between_classes` is a full square matrix.
    pub fn new(
        class_of_net: Vec<NetClassId>,
        default_class: NetClassId,
        track_width_of_class: Vec<i32>,
        via_kinds_of_class: Vec<Vec<ViaKindId>>,
        clearance_between_classes: Vec<i32>,
        edge_clearance: i32,
    ) -> Self {
        let classes = track_width_of_class.len();
        assert_eq!(via_kinds_of_class.len(), classes);
        assert_eq!(clearance_between_classes.len(), classes * classes);
        let largest_clearance = clearance_between_classes.iter().copied().max().unwrap_or(0);
        Self {
            class_of_net,
            default_class,
            track_width_of_class,
            via_kinds_of_class,
            clearance_between_classes,
            largest_clearance,
            edge_clearance,
            cutout_clearances: Vec::new(),
            hole_to_hole: 0,
            track_width_limits_of_class: Vec::new(),
            via_limits_of_class: Vec::new(),
        }
    }

    /// How far apart the holes of vias stay.
    #[must_use]
    pub fn with_hole_to_hole(mut self, distance: i32) -> Self {
        self.hole_to_hole = distance;
        self
    }

    pub fn hole_to_hole(&self) -> i32 {
        self.hole_to_hole
    }

    /// How narrow and how wide the tracks of each class may be, in class order.
    #[must_use]
    pub fn with_track_width_limits(mut self, limits: Vec<Limits>) -> Self {
        self.track_width_limits_of_class = limits;
        self
    }

    /// How large across and how large a drill the vias of each class may
    /// have, in class order.
    #[must_use]
    pub fn with_via_limits(mut self, limits: Vec<ViaLimits>) -> Self {
        self.via_limits_of_class = limits;
        self
    }

    /// How large across and how large a drill a via of `net` may have; no
    /// limits where none were given.
    pub fn via_limits(&self, net: NetId) -> ViaLimits {
        self.via_limits_of_class
            .get(self.class(Some(net)).index())
            .copied()
            .unwrap_or_default()
    }

    /// How narrow and how wide a track of `net` may be; no limits where none
    /// were given.
    pub fn track_width_limits(&self, net: NetId) -> Limits {
        self.track_width_limits_of_class
            .get(self.class(Some(net)).index())
            .copied()
            .unwrap_or_default()
    }

    /// The edge clearance of each hole of the outline, in its order.
    #[must_use]
    pub fn with_cutout_clearances(mut self, clearances: Vec<i32>) -> Self {
        self.cutout_clearances = clearances;
        self
    }

    fn class(&self, net: Option<NetId>) -> NetClassId {
        net.map_or(self.default_class, |net| self.class_of_net[net.index()])
    }

    pub fn track_width(&self, net: NetId) -> i32 {
        self.track_width_of_class[self.class(Some(net)).index()]
    }

    pub fn via_kinds(&self, net: NetId) -> &[ViaKindId] {
        &self.via_kinds_of_class[self.class(Some(net)).index()]
    }

    /// The clearance between copper of two nets; `None` is copper of no net.
    pub fn clearance(&self, a: Option<NetId>, b: Option<NetId>) -> i32 {
        let classes = self.track_width_of_class.len();
        self.clearance_between_classes[self.class(a).index() * classes + self.class(b).index()]
    }

    /// The largest clearance between copper of any two nets: how far around
    /// copper others can conflict with it. The board's edge, checked against
    /// the outline, keeps its own.
    pub fn largest_clearance(&self) -> i32 {
        self.largest_clearance
    }

    pub fn edge_clearance(&self) -> i32 {
        self.edge_clearance
    }

    /// The clearance from the `hole`th hole of the outline.
    pub fn cutout_clearance(&self, hole: usize) -> i32 {
        self.cutout_clearances
            .get(hole)
            .copied()
            .unwrap_or(self.edge_clearance)
    }
}
