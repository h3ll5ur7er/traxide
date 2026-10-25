//! What negotiation does on one board, round by round ([[TRX-56]]): how many
//! connections stand routed after each round of as many commits as the board
//! has connections, how many rip-ups the round took, which connections take
//! each other's place most often, and where, drawn over the board.

use anyhow::{Context, Result, bail};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use traxide_contracts::geometry::Point;
use traxide_contracts::{ConnectionId, Design, Event, Plan};
use traxide_events::CancelToken;
use traxide_routing::RouteJob;

/// Rip-ups are counted by squares of this size, in nanometres: 1 mm.
const PLACE: i32 = 1_000_000;

#[derive(Default)]
struct Log {
    total: usize,
    commits: usize,
    /// Per round: connections routed at its end, and rip-ups during it.
    rounds: Vec<(usize, usize)>,
    rip_ups: Vec<(ConnectionId, ConnectionId, Point)>,
}

impl Log {
    fn take(&mut self, event: Event) {
        match event {
            Event::ConnectionRouted { done, total } => {
                self.total = total;
                if self.commits.is_multiple_of(total.max(1)) {
                    self.rounds.push((done, 0));
                }
                self.commits += 1;
                if let Some(round) = self.rounds.last_mut() {
                    round.0 = done;
                }
            }
            Event::RippedUp { connection, by, at } => {
                if let Some(round) = self.rounds.last_mut() {
                    round.1 += 1;
                }
                self.rip_ups.push((connection, by, at));
            }
            Event::StageStarted(_) => {}
        }
    }
}

pub fn run(args: &[String]) -> Result<()> {
    let [board] = args else {
        bail!("usage: cargo xtask negotiation BOARD");
    };
    if cfg!(debug_assertions) {
        eprintln!(
            "Routing in a debug build takes ten times as long: \
             cargo run --release -p xtask -- negotiation BOARD"
        );
    }
    let board = PathBuf::from(board);
    let stem = board.file_stem().context("a board file")?.to_string_lossy();
    let out = Path::new("target/negotiation").join(stem.as_ref());
    std::fs::create_dir_all(&out)?;
    let routes = traxide_board_access::default_routes_path(&board);
    let job = RouteJob {
        input: board.clone(),
        output: Some(out.join(routes.file_name().context("a routes file")?)),
        profile: "default".into(),
        profile_file: None,
        threads: None,
        unrouted_net_classes: Vec::new(),
        drc_report: None,
        time_limit: None,
        reroute_existing: false,
    };
    let log = RefCell::new(Log::default());
    let report = traxide_routing::route(&job, &|e| log.borrow_mut().take(e), &CancelToken::new())?;
    let log = log.into_inner();
    let design = traxide_board_access::load_design(&board)?;
    let plan = traxide_planning::plan(&design);

    println!("| Round | Routed at its end | Rip-ups |");
    println!("| ---: | ---: | ---: |");
    for (round, (routed, rip_ups)) in log.rounds.iter().enumerate() {
        println!("| {} | {routed} of {} | {rip_ups} |", round + 1, log.total);
    }
    println!(
        "\n{} of {} connections routed in {:.1} s, after {} commits and {} rip-ups.",
        report.connections - report.unrouted.len(),
        report.connections,
        report.elapsed.as_secs_f64(),
        log.commits,
        log.rip_ups.len()
    );

    let name = |id: ConnectionId| {
        let c = plan.connections[id.index()];
        format!(
            "{} {}→{}",
            design.net(c.net).name,
            design.pad(c.from).name,
            design.pad(c.to).name
        )
    };
    let mut pairs: BTreeMap<(ConnectionId, ConnectionId), usize> = BTreeMap::new();
    let mut places: BTreeMap<(i32, i32), usize> = BTreeMap::new();
    for &(a, b, at) in &log.rip_ups {
        *pairs.entry((a.min(b), a.max(b))).or_default() += 1;
        *places
            .entry((at.x.div_euclid(PLACE), at.y.div_euclid(PLACE)))
            .or_default() += 1;
    }
    println!("\nConnections that take each other's place most often:");
    for ((a, b), count) in most(&pairs, 10) {
        println!("  {count}× {} and {}", name(a), name(b));
    }
    println!("\nPlaces with the most rip-ups (1 mm squares, corner in mm):");
    for ((x, y), count) in most(&places, 10) {
        println!("  {count}× at ({x}, {y})");
    }
    let picture = out.join("rip-ups.svg");
    std::fs::write(&picture, svg(&design, &plan, &log, &places))?;
    println!("\nDrawn in {}", picture.display());
    Ok(())
}

fn most<K: Copy + Ord>(counts: &BTreeMap<K, usize>, n: usize) -> Vec<(K, usize)> {
    let mut sorted: Vec<(K, usize)> = counts.iter().map(|(&k, &c)| (k, c)).collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    sorted.truncate(n);
    sorted
}

/// The board's outline and pads, the connections still unrouted at the end as
/// lines, and a red square per millimetre where rip-ups happened, the darker
/// the more.
fn svg(design: &Design, plan: &Plan, log: &Log, places: &BTreeMap<(i32, i32), usize>) -> String {
    let bounds = design.outline.bounds();
    let mm = |v: i32| f64::from(v) / 1e6;
    let (x0, y0) = (mm(bounds.min.x), mm(bounds.min.y));
    let (width, height) = (mm(bounds.max.x) - x0, mm(bounds.max.y) - y0);
    let most = places.values().copied().max().unwrap_or(1) as f64;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{x0} {y0} {width} {height}\" \
         width=\"{}\" height=\"{}\">\n<rect x=\"{x0}\" y=\"{y0}\" width=\"{width}\" \
         height=\"{height}\" fill=\"white\" stroke=\"black\" stroke-width=\"0.2\"/>\n",
        width * 10.0,
        height * 10.0
    );
    for pad in &design.pads {
        let _ = writeln!(
            svg,
            "<circle cx=\"{}\" cy=\"{}\" r=\"0.3\" fill=\"#999\"/>",
            mm(pad.position.x),
            mm(pad.position.y)
        );
    }
    for (&(x, y), &count) in places {
        let _ = writeln!(
            svg,
            "<rect x=\"{x}\" y=\"{y}\" width=\"1\" height=\"1\" fill=\"red\" \
             fill-opacity=\"{:.2}\"><title>{count} rip-ups</title></rect>",
            0.15 + 0.85 * count as f64 / most
        );
    }
    let ripped: std::collections::BTreeSet<ConnectionId> =
        log.rip_ups.iter().map(|&(c, _, _)| c).collect();
    for id in ripped {
        let c = plan.connections[id.index()];
        let (a, b) = (design.pad(c.from).position, design.pad(c.to).position);
        let _ = writeln!(
            svg,
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"blue\" \
             stroke-width=\"0.1\" stroke-opacity=\"0.4\"/>",
            mm(a.x),
            mm(a.y),
            mm(b.x),
            mm(b.y)
        );
    }
    svg.push_str("</svg>\n");
    svg
}
