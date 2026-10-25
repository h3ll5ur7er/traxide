//! The real-world test: KiCad builds each board and exports it, traxide routes
//! it, KiCad imports the session and KiCad's own DRC judges the result. In
//! Docker, traxide's KiCad plugin is installed into a running KiCad and routes
//! each board open there when its button is pressed, through KiCad's IPC API.

use crate::kicad::{self, Kicad};
use crate::kicad_package::{self, Platform};
use crate::running_kicad::RunningKicad;
use anyhow::{Context, Result, bail, ensure};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use traxide_contracts::Design;

const DEFAULT_RECIPES: &[&str] = &[
    "placements",
    "crossing",
    "ground-pour",
    "inner-plane",
    "matrix-3x4",
    "isolation",
];

pub fn run(args: &[String]) -> Result<()> {
    let kicad = Kicad::from_args(args);
    let (root, traxide) = kicad::build_traxide()?;
    let package = match kicad.image() {
        Some(_) => Some(plugin_package(&root)?),
        None => None,
    };
    let mut failures = Vec::new();
    for recipe in kicad::recipes(args, DEFAULT_RECIPES) {
        match round_trip(&kicad, &root, &traxide, package.as_deref(), recipe) {
            Ok(summary) => println!("{recipe}: {summary}"),
            Err(error) => {
                println!("{recipe}: FAILED\n{error:#}");
                failures.push(recipe);
            }
        }
    }
    if !failures.is_empty() {
        bail!(
            "{} of the boards failed: {}",
            failures.len(),
            failures.join(", ")
        );
    }
    Ok(())
}

/// The KiCad plugin's package for Linux, as the Binaries workflow builds it.
fn plugin_package(root: &Path) -> Result<PathBuf> {
    let package = root.join("target/e2e/traxide-kicad-linux.zip");
    std::fs::create_dir_all(package.parent().context("a folder")?)?;
    kicad_package::package(
        Platform::Linux,
        &kicad::build_static_traxide(root)?,
        &package,
    )?;
    Ok(package)
}

fn round_trip(
    kicad: &Kicad,
    root: &Path,
    traxide: &Path,
    package: Option<&Path>,
    recipe: &str,
) -> Result<String> {
    let out = root.join("target/e2e").join(recipe);
    std::fs::create_dir_all(&out)?;
    kicad.export(root, &out, recipe)?;
    // Specctra DSN has no words for KiCad's custom rules, so a board that has
    // them is routed from its own file only.
    let custom_rules = out.join(format!("{recipe}.kicad_dru")).is_file();
    let mut summary = String::new();
    if !custom_rules {
        summary = through_dsn(kicad, root, traxide, recipe, &out)?;
    }
    let started = Instant::now();
    let direct = Command::new(traxide)
        .arg("route")
        .arg("--quiet")
        .arg(out.join(format!("{recipe}.kicad_pcb")))
        .arg("-o")
        .arg(out.join(format!("{recipe}.direct.kicad_pcb")))
        .output()?;
    let seconds = started.elapsed().as_secs_f64();
    ensure!(
        direct.status.success(),
        "traxide did not route the KiCad board completely:\n{}{}",
        String::from_utf8_lossy(&direct.stdout),
        String::from_utf8_lossy(&direct.stderr)
    );
    let from_board = kicad.judge_board(root, &out, &format!("{recipe}.direct"))?;
    ensure!(
        from_board.is_clean(),
        "routed from the board file, KiCad's DRC found {} violations and {} unconnected items:\n  {}",
        from_board.violations.len(),
        from_board.unconnected.len(),
        [from_board.violations, from_board.unconnected]
            .concat()
            .join("\n  ")
    );
    let in_kicad = match package {
        Some(package) => through_running_kicad(kicad, root, package, recipe, &out)?,
        None => "not routed in a running KiCad, which needs Docker".into(),
    };
    let how = if custom_rules {
        format!(
            "routed from the board file by its custom rules in {seconds:.2} s, clean in KiCad's DRC ({})",
            first_line(&direct.stdout)
                .split(" → ")
                .next()
                .unwrap_or_default()
        )
    } else {
        format!("clean in KiCad's DRC from the board file too; {summary}")
    };
    Ok(format!("{how}; {in_kicad}"))
}

/// Opens the recipe's board in KiCad with the plugin `package` installed,
/// requires KiCad's API to give the same design as the board file, routes it
/// by pressing the plugin's button, has KiCad save
/// it and judge it with its zones as traxide left them, refilled, then
/// requires the undos to take the routes back: one, or two on a board with
/// copper zones, whose refill is an undo step of its own.
fn through_running_kicad(
    kicad: &Kicad,
    root: &Path,
    package: &Path,
    recipe: &str,
    out: &Path,
) -> Result<String> {
    let image = kicad.image().context("KiCad in Docker")?;
    let running = RunningKicad::open(
        image,
        &out.join(format!("{recipe}.kicad_pcb")),
        &out.join("in-kicad"),
        package,
    )?;
    let before = running.design()?;
    let from_file =
        traxide_board_access::load_design(&running.project.join(format!("{recipe}.kicad_pcb")))?;
    ensure!(
        before == from_file,
        "KiCad's API gives another design than the board file; they differ in {}",
        differences(&before, &from_file).join(", ")
    );
    running.route_with_the_plugin(&before)?;
    running.save()?;
    let judgement = kicad.judge_saved_in_kicad(root, &running.project, recipe)?;
    ensure!(
        judgement.is_clean() && judgement.tracks > 0,
        "routed in KiCad and saved, KiCad's DRC found {} violations and {} unconnected items in {} tracks:\n  {}",
        judgement.violations.len(),
        judgement.unconnected.len(),
        judgement.tracks,
        [judgement.violations, judgement.unconnected]
            .concat()
            .join("\n  ")
    );
    ensure!(
        judgement.filled_zones == judgement.zones,
        "routed in KiCad, {} of the board's {} copper zones are filled",
        judgement.filled_zones,
        judgement.zones
    );
    let undos = if judgement.zones > 0 {
        running.undo()?;
        // KiCad runs the undo on its GUI thread after answering.
        std::thread::sleep(Duration::from_secs(2));
        ensure!(
            running.design()? != before,
            "the first undo took the routes back rather than the refilled zones"
        );
        "two undos, the first taking back the refill"
    } else {
        "one undo"
    };
    running.undo()?;
    let started = Instant::now();
    while running.design()? != before {
        ensure!(
            started.elapsed() < Duration::from_secs(10),
            "{undos} did not take the routes back out of KiCad"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    let zones = if judgement.zones > 0 {
        ", zones refilled"
    } else {
        ""
    };
    Ok(format!(
        "routed in a running KiCad by its traxide button ({} track segments, {} vias{zones}), clean, and gone after {undos}",
        judgement.tracks, judgement.vias
    ))
}

/// The parts in which two designs differ.
fn differences(a: &Design, b: &Design) -> Vec<&'static str> {
    [
        ("layers", a.layers == b.layers),
        ("outline", a.outline == b.outline),
        ("nets", a.nets == b.nets),
        ("pads", a.pads == b.pads),
        ("via kinds", a.via_kinds == b.via_kinds),
        ("keepouts", a.keepouts == b.keepouts),
        ("existing copper", a.existing == b.existing),
        ("rules", a.rules == b.rules),
    ]
    .into_iter()
    .filter(|(_, same)| !same)
    .map(|(part, _)| part)
    .collect()
}

/// Routes the recipe's DSN export and has KiCad judge the imported session;
/// says how long routing took.
fn through_dsn(
    kicad: &Kicad,
    root: &Path,
    traxide: &Path,
    recipe: &str,
    out: &Path,
) -> Result<String> {
    let started = Instant::now();
    let routed = Command::new(traxide)
        .arg("route")
        .arg("--quiet")
        .arg(out.join(format!("{recipe}.dsn")))
        .output()?;
    let seconds = started.elapsed().as_secs_f64();
    ensure!(
        routed.status.success(),
        "traxide did not route everything:\n{}",
        String::from_utf8_lossy(&routed.stdout)
    );
    let judgement = kicad.judge(
        root,
        out,
        recipe,
        &format!("{recipe}.ses"),
        &format!("{recipe}.routed"),
    )?;
    ensure!(
        judgement.is_clean(),
        "KiCad's DRC found {} violations and {} unconnected items:\n  {}",
        judgement.violations.len(),
        judgement.unconnected.len(),
        [judgement.violations, judgement.unconnected]
            .concat()
            .join("\n  ")
    );
    Ok(format!(
        "routed in {seconds:.2} s ({})",
        first_line(&routed.stdout)
            .split(" → ")
            .next()
            .unwrap_or_default()
    ))
}

fn first_line(output: &[u8]) -> String {
    String::from_utf8_lossy(output)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}
