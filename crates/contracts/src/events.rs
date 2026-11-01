use crate::ConnectionId;
use crate::geometry::Point;

/// What a manager tells its client while a job runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    StageStarted(Stage),
    /// Routing starts round `round` (from 1) of `of`, routing the board
    /// again at other prices, and counts its connections from 0 again.
    RoundStarted {
        round: u32,
        of: u32,
    },
    ConnectionRouted {
        done: usize,
        total: usize,
    },
    /// Negotiation took `connection`'s route out for `by`'s, which met it at `at`.
    RippedUp {
        connection: ConnectionId,
        by: ConnectionId,
        at: Point,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Loading,
    CompilingRules,
    BuildingLayout,
    Planning,
    GlobalRouting,
    Routing,
    Optimising,
    Verifying,
    Storing,
}
