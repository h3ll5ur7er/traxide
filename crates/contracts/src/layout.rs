use crate::{ConnectionId, LayerId, NetId, Routes, Via};
use traxide_geometry::Segment;

/// What stands in the way of new copper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Obstruction {
    None,
    /// Only routed connections, which could be ripped up; sorted, without duplicates.
    Routed(Vec<ConnectionId>),
    /// Something that stays: a pad, a keepout, the board edge, locked copper.
    Fixed,
}

/// The routing state: what copper is where, and whether more fits.
///
/// Engines use it through this trait; the manager chooses the implementation.
pub trait LayoutAccess {
    /// What keeps a track segment of `net` on `layer` from fitting.
    fn track_obstruction(
        &self,
        net: NetId,
        layer: LayerId,
        segment: Segment,
        width: i32,
    ) -> Obstruction;

    /// The routed connections of other nets whose copper a track segment of
    /// `net` on `layer` would run over, as a track crossing them does; not
    /// those it would only come closer to than the clearance, which could
    /// move aside. Sorted, without duplicates.
    fn track_overlaps(
        &self,
        net: NetId,
        layer: LayerId,
        segment: Segment,
        width: i32,
    ) -> Vec<ConnectionId>;

    /// What keeps `via` from fitting on the layers it spans.
    fn via_obstruction(&self, via: &Via) -> Obstruction;

    /// Whether a track segment fits; implementations may answer faster than
    /// [`track_obstruction`](Self::track_obstruction), as they may stop at the first conflict.
    fn is_track_clear(&self, net: NetId, layer: LayerId, segment: Segment, width: i32) -> bool {
        self.track_obstruction(net, layer, segment, width) == Obstruction::None
    }

    fn is_via_clear(&self, via: &Via) -> bool {
        self.via_obstruction(via) == Obstruction::None
    }

    /// Adds the copper routed for `connection`.
    fn add_route(&mut self, connection: ConnectionId, routes: &Routes);

    /// Removes all copper routed for `connection`.
    fn remove_route(&mut self, connection: ConnectionId);
}
