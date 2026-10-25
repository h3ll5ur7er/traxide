//! KiCad as the test harness sees it: it builds boards, exports them as DSN,
//! imports sessions and judges the result with its own DRC.
//!
//! KiCad runs in the official Docker image (`TRAXIDE_KICAD_IMAGE`, by default
//! kicad/kicad:10.0), or, with `--local`, as installed: `TRAXIDE_KICAD_PYTHON`
//! (a Python that can `import pcbnew`) and `TRAXIDE_KICAD_CLI`.

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

pub enum Kicad {
    Docker { image: String },
    Local { python: String, cli: String },
}

/// What KiCad found on a routed board.
#[derive(Debug)]
pub struct Judgement {
    pub violations: Vec<String>,
    /// How many violations of each kind, such as `clearance` or `hole_clearance`.
    pub violation_kinds: Vec<(String, usize)>,
    pub unconnected: Vec<String>,
    pub tracks: usize,
    pub vias: usize,
    pub length_mm: f64,
    /// The copper zones, and those of them that KiCad filled.
    pub zones: usize,
    pub filled_zones: usize,
}

impl Judgement {
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty() && self.unconnected.is_empty()
    }
}

#[derive(Debug, Deserialize)]
struct DrcReport {
    violations: Vec<Finding>,
    unconnected_items: Vec<Finding>,
}

#[derive(Debug, Deserialize)]
struct Finding {
    #[serde(rename = "type")]
    kind: String,
    description: String,
    #[serde(default)]
    items: Vec<FindingItem>,
}

#[derive(Debug, Deserialize)]
struct FindingItem {
    description: String,
}

impl Finding {
    fn line(&self) -> String {
        let items: Vec<&str> = self.items.iter().map(|i| i.description.as_str()).collect();
        format!("{}: {}", self.description, items.join(" / "))
    }
}

#[derive(Debug, Deserialize)]
struct Imported {
    tracks: usize,
    vias: usize,
    length_mm: f64,
    zones: usize,
    filled_zones: usize,
}

impl Kicad {
    pub fn from_args(args: &[String]) -> Self {
        if args.iter().any(|a| a == "--local") {
            Self::Local {
                python: std::env::var("TRAXIDE_KICAD_PYTHON").unwrap_or_else(|_| "python3".into()),
                cli: std::env::var("TRAXIDE_KICAD_CLI").unwrap_or_else(|_| "kicad-cli".into()),
            }
        } else {
            Self::Docker {
                image: std::env::var("TRAXIDE_KICAD_IMAGE")
                    .unwrap_or_else(|_| "kicad/kicad:10.0".into()),
            }
        }
    }

    /// The Docker image KiCad runs from, unless it runs as installed.
    pub fn image(&self) -> Option<&str> {
        match self {
            Self::Docker { image } => Some(image),
            Self::Local { .. } => None,
        }
    }

    /// Builds `recipe` and exports it to `out/RECIPE.dsn`, next to the board.
    pub fn export(&self, root: &Path, out: &Path, recipe: &str) -> Result<()> {
        self.run(
            root,
            out,
            &format!("python3 roundtrip.py export {recipe} {{out}}"),
        )
        .map(drop)
    }

    /// Exports the saved board `out/BOARD.kicad_pcb` to `out/BOARD.dsn`.
    pub fn export_dsn(&self, root: &Path, out: &Path, board: &str) -> Result<()> {
        self.run(
            root,
            out,
            &format!("python3 roundtrip.py dsn {{out}}/{board}.kicad_pcb"),
        )
        .map(drop)
    }

    /// Imports `out/SESSION` into `out/BOARD` and saves the result as `out/ROUTED`.
    pub fn import(
        &self,
        root: &Path,
        out: &Path,
        board: &str,
        session: &str,
        routed: &str,
    ) -> Result<()> {
        self.run(
            root,
            out,
            &format!(
                "python3 roundtrip.py import {{out}}/{board} {{out}}/{session} {{out}}/{routed}"
            ),
        )
        .map(drop)
    }

    /// Imports `out/SESSION` into the recipe's board as `out/ROUTED.kicad_pcb` and runs KiCad's DRC on it.
    pub fn judge(
        &self,
        root: &Path,
        out: &Path,
        recipe: &str,
        session: &str,
        routed: &str,
    ) -> Result<Judgement> {
        let report = format!("{routed}.drc.json");
        let printed = self.run(
            root,
            out,
            &format!(
                "python3 roundtrip.py import {{out}}/{recipe}.kicad_pcb {{out}}/{session} {{out}}/{routed}.kicad_pcb && \
                 python3 roundtrip.py refill {{out}}/{routed}.kicad_pcb && \
                 kicad-cli pcb drc --format json --severity-error -o {{out}}/{report} {{out}}/{routed}.kicad_pcb"
            ),
        )?;
        judged(&printed, &out.join(report))
    }

    /// Loads `out/ROUTED.kicad_pcb`, routed without KiCad, as KiCad opens it,
    /// and runs KiCad's DRC on it with every zone refilled, as a designer
    /// refills them before checking: a board file keeps the fills its zones
    /// came with, poured round the copper before routing.
    pub fn judge_board(&self, root: &Path, out: &Path, routed: &str) -> Result<Judgement> {
        let report = format!("{routed}.drc.json");
        let printed = self.run(
            root,
            out,
            &format!(
                "python3 roundtrip.py stats {{out}}/{routed}.kicad_pcb && \
                 kicad-cli pcb drc --format json --severity-error --refill-zones -o {{out}}/{report} {{out}}/{routed}.kicad_pcb"
            ),
        )?;
        judged(&printed, &out.join(report))
    }

    /// Runs KiCad's DRC on `out/ROUTED.kicad_pcb` as a running KiCad saved
    /// it, with the zones it refilled: only zones left unfilled are filled.
    pub fn judge_saved_in_kicad(&self, root: &Path, out: &Path, routed: &str) -> Result<Judgement> {
        let report = format!("{routed}.drc.json");
        let printed = self.run(
            root,
            out,
            &format!(
                "python3 roundtrip.py stats {{out}}/{routed}.kicad_pcb && \
                 python3 roundtrip.py refill {{out}}/{routed}.kicad_pcb && \
                 kicad-cli pcb drc --format json --severity-error -o {{out}}/{report} {{out}}/{routed}.kicad_pcb"
            ),
        )?;
        judged(&printed, &out.join(report))
    }

    /// Runs `script` (a shell command line) in the e2e scripts' folder; `{out}` stands for `out`.
    fn run(&self, root: &Path, out: &Path, script: &str) -> Result<String> {
        let mut command = match self {
            Self::Docker { image } => {
                let mut command = Command::new("docker");
                command.args(["run", "--rm", "-e", "HOME=/tmp", "-w", "/work/e2e/kicad"]);
                if let Some(user) = current_user() {
                    command.args(["--user", &user]);
                }
                command
                    .arg("-v")
                    .arg(format!("{}:/work:ro", root.display()))
                    .arg("-v")
                    .arg(format!("{}:/out", out.display()))
                    .args([image.as_str(), "sh", "-c", &script.replace("{out}", "/out")]);
                command
            }
            Self::Local { python, cli } => {
                let script = script
                    .replace("{out}", &out.display().to_string())
                    .replace("python3 ", &format!("{python} "))
                    .replace("kicad-cli ", &format!("{cli} "));
                let mut command = Command::new("sh");
                command
                    .args(["-c", &script])
                    .current_dir(root.join("e2e/kicad"));
                command
            }
        };
        let output = command.output().context("running KiCad")?;
        let text = String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr);
        ensure!(output.status.success(), "KiCad failed: {script}\n{text}");
        Ok(text)
    }
}

/// The statistics a KiCad script printed as JSON, and the DRC report at `report`.
fn judged(printed: &str, report: &Path) -> Result<Judgement> {
    let imported: Imported = printed
        .lines()
        .find_map(|line| serde_json::from_str(line).ok())
        .context("the import printed no statistics")?;
    let drc: DrcReport = serde_json::from_str(&std::fs::read_to_string(report)?)?;
    let mut kinds = std::collections::BTreeMap::new();
    for finding in &drc.violations {
        *kinds.entry(finding.kind.clone()).or_insert(0) += 1;
    }
    Ok(Judgement {
        violations: drc.violations.iter().map(Finding::line).collect(),
        violation_kinds: kinds.into_iter().collect(),
        unconnected: drc.unconnected_items.iter().map(Finding::line).collect(),
        tracks: imported.tracks,
        vias: imported.vias,
        length_mm: imported.length_mm,
        zones: imported.zones,
        filled_zones: imported.filled_zones,
    })
}

/// So that files KiCad writes into the mounted folder belong to the caller.
fn current_user() -> Option<String> {
    let id = |flag| {
        Command::new("id")
            .arg(flag)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    Some(format!("{}:{}", id("-u")?, id("-g")?))
}

/// The repository's root and a release build of `traxide` in it.
pub fn build_traxide() -> Result<(PathBuf, PathBuf)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()?;
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let built = Command::new(&cargo)
        .args(["build", "--release", "--quiet", "-p", "traxide-cli"])
        .current_dir(&root)
        .status()?;
    ensure!(built.success(), "building traxide failed");
    let traxide = root.join("target/release/traxide");
    Ok((root, traxide))
}

/// A static build of `traxide` in the repository at `root`, for the KiCad
/// plugin: it runs whatever the C library of the Linux it lands on.
pub fn build_static_traxide(root: &Path) -> Result<PathBuf> {
    let target = "x86_64-unknown-linux-musl";
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let built = Command::new(&cargo)
        .args(["build", "--release", "--quiet", "-p", "traxide-cli"])
        .args(["--target", target])
        .current_dir(root)
        .status()?;
    ensure!(
        built.success(),
        "building the static traxide failed: rustup target add {target}?"
    );
    Ok(root.join("target").join(target).join("release/traxide"))
}

/// The recipes named in `args`, or `default` when none is.
pub fn recipes<'a>(args: &'a [String], default: &[&'a str]) -> Vec<&'a str> {
    let named: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(String::as_str)
        .collect();
    if named.is_empty() {
        default.to_vec()
    } else {
        named
    }
}
