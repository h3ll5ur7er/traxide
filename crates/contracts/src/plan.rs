use crate::{DifferentialPair, NetId, PadId};

/// The connections to route, in the order to route them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub connections: Vec<Connection>,
}

/// Two pads of one net that routing joins with copper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Connection {
    pub net: NetId,
    pub from: PadId,
    pub to: PadId,
}

/// The connections of a differential pair that run side by side: each starts
/// beside the other and ends beside it, so they can be routed as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoupledConnections {
    pub pair: DifferentialPair,
    /// The positive net's connection, then the negative's, whose `from` lies
    /// beside the positive's `from`.
    pub connections: [Connection; 2],
}
