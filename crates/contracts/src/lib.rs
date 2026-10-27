//! What traxide's components agree on: the data that flows between them and the
//! traits through which engines reach resource access.
//!
//! Lengths and coordinates are nanometres (see `traxide-geometry`). Ids are dense
//! indices into the arrays of a [`Design`].

mod constraints;
mod corridors;
mod design;
mod events;
mod ids;
mod layout;
mod plan;
mod profile;
mod routes;
#[cfg(feature = "testing")]
pub mod testing;
mod verdict;

pub use constraints::{ConstraintSet, ViaLimits};
pub use corridors::Corridors;
pub use design::{
    Condition, Constraint, CustomRule, Design, DesignRules, Item, Keepout, KeepoutScope, Layer,
    LayerShape, Limits, Net, NetClass, Pad, Plane, ViaKind,
};
pub use events::{Event, Stage};
pub use ids::{ConnectionId, LayerId, NetClassId, NetId, PadId, ViaKindId};
pub use layout::{LayoutAccess, Obstruction};
pub use plan::{Connection, Plan};
pub use profile::Profile;
pub use routes::{Routes, RoutingResult, Track, Via};
pub use traxide_geometry as geometry;
pub use verdict::{CopperItem, Verdict, Violation};
