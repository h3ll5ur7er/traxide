/// How to trade routing speed against quality.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: String,
    /// Spacing of the routing grid.
    pub grid_pitch: i32,
    /// What a via costs, in nanometres of track it is worth avoiding.
    pub via_cost: i64,
    /// What a 45° change of direction costs, in nanometres of track.
    pub bend_cost: i64,
    /// How far a connection may stray beyond the box around its pads.
    pub detour_margin: i32,
    /// What crossing a routed connection costs, in nanometres of track, when a
    /// blocked connection looks for routes to rip up. It grows with each time
    /// that connection has been ripped up before.
    pub rip_up_cost: i64,
    /// How often one connection may be ripped up before it stays unrouted, so
    /// that two connections cannot take each other's place forever.
    pub max_rip_ups: u32,
    /// What each collision adds to its place for every later search, in
    /// nanometres of track per place crossed (negotiated congestion's history
    /// cost): connections learn to leave a contested place to one of them
    /// instead of taking turns ripping each other up there. Zero turns it off.
    pub history_cost: i64,
    /// How strongly a search heads for its target: 1 finds the cheapest path;
    /// more finds a path within that factor of the cheapest, much faster.
    pub heuristic_weight: f64,
    /// Whether global routing first plans a corridor for every connection, to
    /// which detailed routing then keeps.
    pub global_routing: bool,
    /// At most how many connections, far enough apart not to meet, are searched
    /// side by side before they are committed in order. Unlike the number of
    /// threads, it can change the result, as it changes the order in which
    /// rip-ups queue connections again.
    pub batch_size: u32,
    /// What a step across its layer's direction costs, as a share of its
    /// length; a diagonal step pays half. The board is routed side by side
    /// without directions and with its layers running crosswise in either
    /// orientation, and again with dearer vias where time is left; the routing
    /// with fewer vias stays. Zero routes once, with no directions.
    pub against_layer_direction: f64,
}
