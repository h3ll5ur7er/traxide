//! Board Access: where boards come from and where routes go (volatility V10 of
//! docs/Architecture/Volatilities.md). Callers see format-neutral contracts; the
//! file extension picks the format, and [`RUNNING_KICAD`] names the board open
//! in KiCad.

mod error;
pub mod kicad;
mod sexp;
pub mod specctra;

pub use error::BoardError;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use traxide_contracts::{Design, Routes, Verdict};

/// Where a path would name a board: the board open in a running KiCad's PCB
/// editor, reached through KiCad's IPC API. Its routes go back into KiCad as
/// one commit, which one undo removes.
pub const RUNNING_KICAD: &str = "kicad://current";

fn is_running_kicad(path: &Path) -> bool {
    path == Path::new(RUNNING_KICAD)
}

pub fn load_design(path: &Path) -> Result<Design, BoardError> {
    let mut design = read_design(path)?;
    design.warnings.extend(parts_outside(&design));
    Ok(design)
}

/// Says which parts lie wholly outside the board's outline, whose pads no
/// track can reach: whole boards of the corpus were exported so.
fn parts_outside(design: &Design) -> Option<String> {
    let mut inside_or_not: BTreeMap<&str, bool> = BTreeMap::new();
    for pad in &design.pads {
        *inside_or_not.entry(pad.part()).or_insert(false) |= design.outline.contains(pad.position);
    }
    let outside: Vec<&str> = inside_or_not
        .into_iter()
        .filter(|&(_, inside)| !inside)
        .map(|(part, _)| part)
        .collect();
    match outside.as_slice() {
        [] => None,
        [part] => Some(format!("the part {part} lies outside the board's outline")),
        parts => Some(format!(
            "the parts {} lie outside the board's outline",
            parts.join(", ")
        )),
    }
}

fn read_design(path: &Path) -> Result<Design, BoardError> {
    if is_running_kicad(path) {
        return kicad::ipc::load_design(&kicad::ipc::Environment::of_process());
    }
    match extension(path) {
        Some("dsn") => specctra::parse_dsn(&read(path)?),
        Some("kicad_pcb") => {
            let beside = Beside::of(path)?;
            kicad::parse_kicad_pcb(&read(path)?, beside.rule_files())
        }
        _ => Err(BoardError::UnsupportedFormat {
            path: path.into(),
            expected: "a Specctra design (.dsn) or a KiCad board (.kicad_pcb)",
        }),
    }
}

/// The names of the nets of what is selected in the board open in KiCad: the
/// copper selected and the selected footprints' pads.
pub fn nets_selected_in_kicad() -> Result<Vec<String>, BoardError> {
    kicad::ipc::selected_nets(&kicad::ipc::Environment::of_process())
}

/// Where the routes of the board at `board` go unless told otherwise: a session
/// beside a Specctra design, and a copy beside a KiCad board, which KiCad opens
/// with the board's project; and the running KiCad for its board.
pub fn default_routes_path(board: &Path) -> PathBuf {
    if is_running_kicad(board) {
        return board.into();
    }
    match extension(board) {
        Some("kicad_pcb") => board.with_extension("routed.kicad_pcb"),
        _ => board.with_extension("ses"),
    }
}

/// What storing routes did besides adding them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stored {
    /// The running KiCad refilled the board's copper zones around the routes,
    /// an undo step of its own: taking the routes back takes two undos.
    pub zones_refilled: bool,
    /// The board file keeps the fills its copper zones came with, poured
    /// round the copper before routing: the designer refills them in KiCad.
    pub zones_to_refill: bool,
}

/// Stores `routes`, the copper routing added to `design`, which was read from
/// `board`, into `path`.
pub fn store_routes(
    path: &Path,
    board: &Path,
    design: &Design,
    routes: &Routes,
) -> Result<Stored, BoardError> {
    if is_running_kicad(path) && is_running_kicad(board) {
        return kicad::ipc::store_routes(&kicad::ipc::Environment::of_process(), design, routes);
    }
    let (text, zones_to_refill) = match (extension(path), extension(board)) {
        (Some("ses"), _) => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            // KiCad's session import replaces every track of the board with the
            // session's, but for the locked: the session carries the board's
            // own copper too, without what is locked, which would come twice.
            let mut everything = Routes {
                tracks: (design.existing.tracks.iter())
                    .filter(|t| !design.locked.tracks.contains(t))
                    .cloned()
                    .collect(),
                vias: (design.existing.vias.iter())
                    .filter(|v| !design.locked.vias.contains(v))
                    .cloned()
                    .collect(),
            };
            everything.extend(routes.clone());
            (specctra::write_ses(&name, design, &everything), false)
        }
        (Some("kicad_pcb"), Some("kicad_pcb")) => {
            let beside = Beside::of(board)?;
            let text = read(board)?;
            let routed = match kicad::add_routes(&text, beside.rule_files(), design, routes)? {
                Some(routed) => routed,
                // A design that routes the board's unlocked wiring again holds
                // only its locked copper: the board without the rest.
                None => kicad::add_routes(
                    &kicad::without_unlocked_wiring(&text),
                    beside.rule_files(),
                    design,
                    routes,
                )?
                .ok_or_else(|| BoardError::Changed { path: board.into() })?,
            };
            // KiCad finds a board's rules in the files of the same name.
            for (extension, rules) in [
                (PROJECT, &beside.project),
                (CUSTOM_RULES, &beside.custom_rules),
            ] {
                let copy = path.with_extension(extension);
                if let Some(rules) = rules
                    && copy != board.with_extension(extension)
                {
                    write(&copy, rules)?;
                }
            }
            (routed.text, routed.has_copper_zones)
        }
        _ => {
            return Err(BoardError::UnsupportedFormat {
                path: path.into(),
                expected: "a Specctra session (.ses), a KiCad board (.kicad_pcb) for a KiCad board, or the running KiCad for its board",
            });
        }
    };
    write(path, &text)?;
    Ok(Stored {
        zones_refilled: false,
        zones_to_refill,
    })
}

/// Stores `verdict`, found on `routes` of `design` in the file `source`, into
/// `path` as a DRC report in the JSON format of `kicad-cli pcb drc`.
pub fn store_drc_report(
    path: &Path,
    source: &Path,
    design: &Design,
    routes: &Routes,
    verdict: &Verdict,
) -> Result<(), BoardError> {
    let source = if is_running_kicad(source) {
        RUNNING_KICAD.to_owned()
    } else {
        source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let report = kicad::drc::report(&source, SystemTime::now(), design, routes, verdict);
    write(path, &format!("{report:#}\n"))
}

fn write(path: &Path, text: &str) -> Result<(), BoardError> {
    std::fs::write(path, text).map_err(|source| BoardError::Io {
        path: path.into(),
        source,
    })
}

/// Reads routes that were stored for `design`.
pub fn load_routes(path: &Path, design: &Design) -> Result<Routes, BoardError> {
    match extension(path) {
        Some("ses") => specctra::parse_ses(&read(path)?, design),
        _ => Err(BoardError::UnsupportedFormat {
            path: path.into(),
            expected: "a Specctra session (.ses)",
        }),
    }
}

const PROJECT: &str = "kicad_pro";
const CUSTOM_RULES: &str = "kicad_dru";

/// The texts of the files next to a KiCad board that hold its rules.
#[derive(Default)]
struct Beside {
    project: Option<String>,
    custom_rules: Option<String>,
}

impl Beside {
    fn of(board: &Path) -> Result<Self, BoardError> {
        let text = |extension| {
            let path = board.with_extension(extension);
            path.is_file().then(|| read(&path)).transpose()
        };
        Ok(Self {
            project: text(PROJECT)?,
            custom_rules: text(CUSTOM_RULES)?,
        })
    }

    fn rule_files(&self) -> kicad::RuleFiles<'_> {
        kicad::RuleFiles {
            project: self.project.as_deref(),
            custom_rules: self.custom_rules.as_deref(),
        }
    }
}

/// What the end-to-end test does with a KiCad it started itself, whose socket
/// and project folder it names, as a designer and a plugin would.
#[cfg(feature = "testing")]
pub mod testing {
    use crate::BoardError;
    use crate::kicad::ipc::{self, Environment};
    use std::path::Path;
    use traxide_contracts::Design;

    fn environment(socket: &Path, project_folder: Option<&Path>) -> Environment {
        Environment {
            socket: socket.into(),
            token: String::new(),
            project_folder: project_folder.map(Path::to_path_buf),
        }
    }

    /// The design of the board open in the KiCad listening on `socket`.
    pub fn load_design_in_kicad(
        socket: &Path,
        project_folder: &Path,
    ) -> Result<Design, BoardError> {
        ipc::load_design(&environment(socket, Some(project_folder)))
    }

    /// Saves the board open in KiCad to its file, as *File → Save* does.
    pub fn save_board_in_kicad(socket: &Path) -> Result<(), BoardError> {
        ipc::testing::save_board(&environment(socket, None))
    }

    /// Selects a footprint with a pad of a net in the board open in KiCad, as
    /// clicking it does, and names it by its reference.
    pub fn select_a_footprint_in_kicad(socket: &Path) -> Result<String, BoardError> {
        ipc::testing::select_a_footprint(&environment(socket, None))
    }

    /// Undoes the last change to the board open in KiCad, as *Edit → Undo* does.
    pub fn undo_in_kicad(socket: &Path) -> Result<(), BoardError> {
        ipc::testing::undo(&environment(socket, None))
    }

    /// Runs the PCB editor's action named `action`, as pressing its button does.
    pub fn run_action_in_kicad(socket: &Path, action: &str) -> Result<(), BoardError> {
        ipc::testing::run_action(&environment(socket, None), action)
    }
}

fn extension(path: &Path) -> Option<&str> {
    path.extension().and_then(|e| e.to_str())
}

/// The file's text: UTF-8, as KiCad writes, or else Latin-1, as some older
/// tools do, which maps every byte to the character of the same number.
fn read(path: &Path) -> Result<String, BoardError> {
    let bytes = std::fs::read(path).map_err(|source| BoardError::Io {
        path: path.into(),
        source,
    })?;
    Ok(String::from_utf8(bytes)
        .unwrap_or_else(|error| error.into_bytes().iter().map(|&b| char::from(b)).collect()))
}
