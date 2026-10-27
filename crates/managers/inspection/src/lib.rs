//! Inspection Manager: the workflows that check and compare boards
//! (volatility V3 of docs/Architecture/Volatilities.md).

use std::collections::BTreeMap;
use std::path::PathBuf;
use traxide_board_access::BoardError;
use traxide_contracts::{ConstraintSet, Design, Violation, geometry::Point};
use traxide_rules::Purpose;

/// Check the board in `board`, with the routes of `session` if given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckJob {
    pub board: PathBuf,
    pub session: Option<PathBuf>,
    /// Where to write a DRC report of the check in KiCad's JSON format, if
    /// anywhere.
    pub drc_report: Option<PathBuf>,
}

/// What a check found: how complete the board is and which rules its copper
/// breaks, judged by the Verification Engine alone.
#[derive(Debug, Clone)]
pub struct CheckReport {
    pub nets: usize,
    /// Nets whose pads copper does not all join.
    pub incomplete_nets: usize,
    /// Pads not joined to their net's first pad.
    pub unconnected_pads: usize,
    /// Every broken rule but unconnected pads.
    pub violations: Vec<Violation>,
    pub tracks: usize,
    pub vias: usize,
    pub track_length_mm: f64,
    /// What the reader found wrong with the board and read anyway.
    pub warnings: Vec<String>,
}

impl CheckReport {
    /// The violations counted by kind, in a stable order.
    pub fn violations_by_kind(&self) -> Vec<(&'static str, usize)> {
        let mut counts = BTreeMap::new();
        for violation in &self.violations {
            *counts.entry(kind(violation)).or_insert(0) += 1;
        }
        counts.into_iter().collect()
    }
}

fn kind(violation: &Violation) -> &'static str {
    match violation {
        Violation::Clearance { .. } => "clearance",
        Violation::HoleToHole { .. } => "hole to hole",
        Violation::TrackWidth { .. } => "track width",
        Violation::ViaDiameter { .. } => "via diameter",
        Violation::HoleSize { .. } => "hole size",
        Violation::Edge { .. } => "edge",
        Violation::Keepout { .. } => "keepout",
        Violation::Dangling { .. } => "dangling",
        Violation::Unconnected { .. } => "unconnected",
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error(transparent)]
    Board(#[from] BoardError),
}

/// Checks the board's copper: the session's, which replaces the board's own
/// but its locked wiring as KiCad's session import does, or else the copper
/// the board came with.
pub fn check(job: &CheckJob) -> Result<CheckReport, CheckError> {
    let design = traxide_board_access::load_design(&job.board)?;
    let routes = match &job.session {
        Some(session) => {
            let mut routes = design.locked.clone();
            routes.extend(traxide_board_access::load_routes(session, &design)?);
            routes
        }
        None => design.existing.clone(),
    };
    let rules = traxide_rules::compile(&design, Purpose::Verification);
    let verdict = traxide_verification::verify(&design, &rules, &routes);
    if let Some(drc_report) = &job.drc_report {
        let judged = job.session.as_ref().unwrap_or(&job.board);
        traxide_board_access::store_drc_report(drc_report, judged, &design, &routes, &verdict)?;
    }
    let (unconnected, violations): (Vec<Violation>, Vec<Violation>) = verdict
        .violations
        .into_iter()
        .partition(|v| matches!(v, Violation::Unconnected { .. }));
    let unconnected_pads = unconnected
        .iter()
        .map(|v| match v {
            Violation::Unconnected { pads, .. } => pads.len(),
            _ => 0,
        })
        .sum();
    let length = |path: &[Point]| -> i64 { path.windows(2).map(|p| p[0].distance(p[1])).sum() };
    Ok(CheckReport {
        nets: design.nets.len(),
        incomplete_nets: unconnected.len(),
        unconnected_pads,
        violations,
        tracks: routes.tracks.len(),
        vias: routes.vias.len(),
        track_length_mm: routes.tracks.iter().map(|t| length(&t.path)).sum::<i64>() as f64 / 1e6,
        warnings: [design.warnings.clone(), pads_too_close(&design, &rules)].concat(),
    })
}

/// Warns of each pair of pads of different nets closer than their clearance,
/// which verification leaves to the footprints and no routing can fix.
fn pads_too_close(design: &Design, rules: &ConstraintSet) -> Vec<String> {
    let named = |pad| {
        let pad = design.pad(pad);
        let net = pad.net.map_or("", |net| design.net(net).name.as_str());
        format!("{} (net {net})", pad.name)
    };
    traxide_verification::pads_too_close(design, rules)
        .into_iter()
        .map(|[a, b]| {
            format!(
                "the pads {} and {} are closer than their clearance",
                named(a),
                named(b)
            )
        })
        .collect()
}
