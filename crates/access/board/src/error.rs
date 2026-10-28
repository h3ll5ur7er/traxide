use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum BoardError {
    #[error("cannot access {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: unsupported format, expected {expected}")]
    UnsupportedFormat {
        path: PathBuf,
        expected: &'static str,
    },
    #[error("line {line}: {message}")]
    Syntax { line: usize, message: String },
    #[error("line {line}: {message}")]
    Invalid { line: usize, message: String },
    #[error("{path} changed since it was read; route it again")]
    Changed { path: PathBuf },
    #[error("the board's project: {0}")]
    Project(String),
    #[error("cannot reach KiCad at {}: {reason}.\n{HOW_TO_REACH_KICAD}", socket.display())]
    KicadUnreachable { socket: PathBuf, reason: String },
    #[error("KiCad: {0}")]
    Kicad(String),
    #[error(
        "nothing of a net is selected in KiCad: select the tracks, vias, pads, footprints \
         or zones whose nets to route"
    )]
    NothingSelected,
}

const HOW_TO_REACH_KICAD: &str = "\
traxide talks to the board open in KiCad's PCB editor through KiCad's API, which is off by default:
  1. In KiCad, tick Preferences → Preferences… → Plugins → Enable KiCad API.
  2. Restart the PCB editor and open the board.
Started from a terminal, traxide looks for KiCad's socket in the temporary folder;
set KICAD_API_SOCKET to the one KiCad created if it is elsewhere.";
