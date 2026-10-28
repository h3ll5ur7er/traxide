//! Freerouting's command line, so that tools and scripts that run Freerouting
//! can run traxide instead (docs/Research/Prior-art.md#interfaces-worth-matching).
//! Implemented from Freerouting's documentation of its arguments.

use anyhow::{Context, Result, bail};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use traxide_routing::RouteJob;

#[derive(Debug, PartialEq)]
pub(crate) enum Invocation {
    Help,
    Route {
        /// Boxed: a job is far larger than the help, above all where paths are
        /// large, as on Windows.
        job: Box<RouteJob>,
        /// Options traxide understands but does not follow, said once each.
        warnings: Vec<String>,
    },
}

pub(crate) const HELP: &str = "\
traxide, called as Freerouting: route a Specctra DSN into a session (SES).

  -de BOARD.dsn       the board; '+'-joined .ses and .rules files are ignored
  -do BOARD.ses       where to write the session; next to the board by default
  -mt N               threads to route on
  -inc CLASS,CLASS    net classes to leave unrouted, such as those of planes
  -drc REPORT.json    a DRC report of the routed board, in KiCad's JSON format

Options that tune Freerouting's own algorithm (-mp, -us, -hr, -is, -im,
--router.*) are ignored with a warning: traxide's profiles decide instead.
GUI, logging, language and analytics options are ignored. traxide's own
command line: traxide route --help";

/// Options of Freerouting's that tune its own algorithm, and take a value.
const ALGORITHM_OPTIONS: &[&str] = &["-mp", "-us", "-hr", "-is", "-oit"];
/// Options about Freerouting's window, logs, language and analytics, which
/// mean nothing to traxide, and take a value.
const QUIET_OPTIONS: &[&str] = &["-di", "-l", "-dct", "-host", "-ll"];
const QUIET_FLAGS: &[&str] = &["-da", "-dl"];
const QUIET_SETTINGS: &[&str] = &["--gui.", "--logging.", "--debug.", "--user_data_path="];

pub(crate) fn parse(args: &[String]) -> Result<Invocation> {
    let mut input = None;
    let mut output = None;
    let mut threads = None;
    let mut unrouted_net_classes = Vec::new();
    let mut drc_report = None;
    let mut warnings = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || args.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "-help" | "--help" | "-h" => return Ok(Invocation::Help),
            "-de" => input = Some(design_input(value()?, &mut warnings)?),
            "-do" => output = Some(session_output(value()?)?),
            "-mt" => threads = NonZeroUsize::new(value()?.parse().context("-mt needs a number")?),
            "-inc" => unrouted_net_classes.extend(
                value()?
                    .split(',')
                    .map(str::trim)
                    .filter(|class| !class.is_empty())
                    .map(String::from),
            ),
            "-dr" => {
                value()?;
                warnings.push("-dr: rules files are not read; the DSN's rules apply".into());
            }
            "-drc" => drc_report = Some(PathBuf::from(value()?)),
            "-im" => warnings.push("-im: ignored, traxide's profiles decide how it routes".into()),
            option if ALGORITHM_OPTIONS.contains(&option) => {
                value()?;
                warnings.push(format!(
                    "{option}: ignored, traxide's profiles decide how it routes"
                ));
            }
            option if QUIET_OPTIONS.contains(&option) => {
                value()?;
            }
            option if QUIET_FLAGS.contains(&option) => {}
            setting if QUIET_SETTINGS.iter().any(|s| setting.starts_with(s)) => {}
            setting if setting.starts_with("--router.") => warnings.push(format!(
                "{setting}: ignored, traxide's profiles decide how it routes"
            )),
            other => warnings.push(format!("{other}: unknown option, ignored")),
        }
    }
    let input: PathBuf = input.context("-de names no board to route")?;
    let output = output.unwrap_or_else(|| input.with_extension("ses"));
    Ok(Invocation::Route {
        job: Box::new(RouteJob {
            input,
            output: Some(output),
            profile: "default".into(),
            profile_file: None,
            threads,
            unrouted_net_classes,
            nets: Vec::new(),
            drc_report,
            time_limit: None,
            reroute_existing: false,
        }),
        warnings,
    })
}

/// The board among `-de`'s files, which Freerouting allows to join with '+'.
fn design_input(files: &str, warnings: &mut Vec<String>) -> Result<PathBuf> {
    let mut board = None;
    for file in files.split('+') {
        if has_extension(file, "dsn") {
            board = Some(PathBuf::from(file));
        } else if has_extension(file, "ses") {
            warnings.push(format!(
                "{file}: continuing from a session is not supported yet; routing from scratch"
            ));
        } else {
            warnings.push(format!("{file}: ignored, the DSN's rules apply"));
        }
    }
    board.with_context(|| format!("-de {files} names no .dsn board"))
}

fn session_output(file: &str) -> Result<PathBuf> {
    if !has_extension(file, "ses") {
        bail!("-do {file}: traxide writes sessions (.ses) only");
    }
    Ok(PathBuf::from(file))
}

fn has_extension(file: &str, extension: &str) -> bool {
    std::path::Path::new(file)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(extension))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Result<Invocation> {
        parse(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>())
    }

    fn route(args: &[&str]) -> (RouteJob, Vec<String>) {
        match parsed(args).expect("valid") {
            Invocation::Route { job, warnings } => (*job, warnings),
            Invocation::Help => panic!("routes"),
        }
    }

    #[test]
    fn a_plugin_style_call_becomes_a_route_job_without_warnings() {
        let (job, warnings) = route(&[
            "-de",
            "b.dsn",
            "-do",
            "out/b.ses",
            "-mt",
            "4",
            "--gui.enabled=false",
            "-da",
            "-host",
            "KiCad",
            "--logging.console.level=WARN",
        ]);
        assert_eq!(job.input, PathBuf::from("b.dsn"));
        assert_eq!(job.output, Some(PathBuf::from("out/b.ses")));
        assert_eq!(job.threads, NonZeroUsize::new(4));
        assert_eq!(warnings, Vec::<String>::new());
    }

    #[test]
    fn the_session_goes_next_to_the_board_by_default() {
        let (job, _) = route(&["-de", "boards/b.dsn"]);
        assert_eq!(job.output, Some(PathBuf::from("boards/b.ses")));
    }

    #[test]
    fn of_joined_input_files_the_board_is_routed_and_the_rest_named_in_warnings() {
        let (job, warnings) = route(&["-de", "b.dsn+b.ses+b.rules"]);
        assert_eq!(job.input, PathBuf::from("b.dsn"));
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].starts_with("b.ses:"), "{warnings:?}");
        assert!(warnings[1].starts_with("b.rules:"), "{warnings:?}");
    }

    #[test]
    fn options_that_tune_freeroutings_algorithm_are_ignored_with_a_warning() {
        let (_, warnings) = route(&[
            "-de",
            "b.dsn",
            "-mp",
            "10",
            "-us",
            "global",
            "--router.via_costs=150",
        ]);
        assert_eq!(
            warnings,
            [
                "-mp: ignored, traxide's profiles decide how it routes",
                "-us: ignored, traxide's profiles decide how it routes",
                "--router.via_costs=150: ignored, traxide's profiles decide how it routes",
            ]
        );
    }

    #[test]
    fn net_classes_to_leave_unrouted_come_from_inc() {
        let (job, _) = route(&["-de", "b.dsn", "-inc", "GND, VCC"]);
        assert_eq!(job.unrouted_net_classes, ["GND", "VCC"]);
    }

    #[test]
    fn drc_names_where_the_drc_report_goes() {
        let (job, warnings) = route(&["-de", "b.dsn", "-drc", "out/b.json"]);
        assert_eq!(job.drc_report, Some(PathBuf::from("out/b.json")));
        assert_eq!(warnings, Vec::<String>::new());
    }

    #[test]
    fn zero_threads_means_all_of_them() {
        let (job, _) = route(&["-de", "b.dsn", "-mt", "0"]);
        assert_eq!(job.threads, None);
    }

    #[test]
    fn unknown_options_are_named_in_a_warning() {
        let (_, warnings) = route(&["-de", "b.dsn", "-xyz"]);
        assert_eq!(warnings, ["-xyz: unknown option, ignored"]);
    }

    #[test]
    fn sessions_are_the_only_output() {
        let error = parsed(&["-de", "b.dsn", "-do", "b.scr"]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "-do b.scr: traxide writes sessions (.ses) only"
        );
    }

    #[test]
    fn a_call_without_a_board_is_an_error() {
        assert!(parsed(&["--gui.enabled=false"]).is_err());
        assert!(parsed(&["-de"]).is_err());
    }

    #[test]
    fn help_is_help() {
        assert_eq!(parsed(&["-help"]).expect("valid"), Invocation::Help);
    }
}
