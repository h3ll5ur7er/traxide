//! KiCad's own files: the board (`.kicad_pcb`), the rules of its project
//! (`.kicad_pro`) and its custom rules (`.kicad_dru`), as KiCad 9 and 10 write
//! them; and a running KiCad, through its IPC API.

mod board;
pub(crate) mod drc;
pub(crate) mod ipc;
mod project;
mod rules;
mod sexpr;
mod unlocked;
mod write;

pub use unlocked::without_unlocked_wiring;

use crate::BoardError;
use traxide_contracts::{Design, Routes};

/// The texts of the files beside a board that hold its rules, where they exist.
#[derive(Debug, Clone, Copy, Default)]
pub struct RuleFiles<'a> {
    /// `.kicad_pro`
    pub project: Option<&'a str>,
    /// `.kicad_dru`
    pub custom_rules: Option<&'a str>,
}

/// The design of the board in `board`, with its rules.
pub fn parse_kicad_pcb(board: &str, rules: RuleFiles) -> Result<Design, BoardError> {
    Ok(board::read(board, rules.project, rules.custom_rules)?.design)
}

/// A board file with routes added.
#[derive(Debug)]
pub struct WithRoutes {
    pub text: String,
    /// Whether the board has zones of copper, whose fills are now out of date.
    pub has_copper_zones: bool,
}

/// The board file `board` with `routes` added, which were routed on `design`;
/// `None` if `board` is no longer the board `design` was read from.
pub fn add_routes(
    board: &str,
    rules: RuleFiles,
    design: &Design,
    routes: &Routes,
) -> Result<Option<WithRoutes>, BoardError> {
    let read = board::read(board, rules.project, rules.custom_rules)?;
    Ok((read.design == *design).then(|| WithRoutes {
        text: write::with_routes(board, &read, routes),
        has_copper_zones: read.has_copper_zones,
    }))
}
