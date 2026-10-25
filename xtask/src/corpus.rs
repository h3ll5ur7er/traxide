//! Routes real boards that other people made and judges the result:
//! `cargo xtask corpus [--timeout SECONDS] [--freerouting CMD] [--shard K/N]
//! [--list FILE]`. Each board is routed by traxide and, when given, by
//! Freerouting; `traxide check` judges both sessions alike, against the rules
//! the board itself already breaks. The boards are downloaded at run time into
//! `target/corpus`, never committed. Prints a Markdown table, and appends it to
//! `$GITHUB_STEP_SUMMARY` in CI. traxide's results land in
//! `target/corpus/results-K-of-N.json`, for `cargo xtask gate` to compare
//! with the baseline.

use crate::gate::{Outcome, Record};
use crate::kicad::{self, Kicad};
use crate::kicad_project::{Project, RULE_FILES, strip_unlocked_copper};
use anyhow::{Context, Result, anyhow, ensure};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEFAULT_LISTS: [&str; 2] = [
    "corpus/freerouting-fixtures.txt",
    "corpus/kicad-projects.txt",
];
const SOURCE: &str = "https://raw.githubusercontent.com/freerouting/freerouting/master/";

pub fn run(args: &[String]) -> Result<()> {
    let timeout = Duration::from_secs(option(args, "--timeout").map_or(Ok(120), |t| t.parse())?);
    let freerouting = option(args, "--freerouting");
    let shard = option(args, "--shard").map_or(Ok((1, 1)), |s| shard(&s))?;
    let (root, traxide) = kicad::build_traxide()?;
    let lists = option(args, "--list").map_or_else(
        || DEFAULT_LISTS.iter().map(|list| root.join(list)).collect(),
        |list| vec![root.join(list)],
    );
    let folder = root.join("target/corpus");
    std::fs::create_dir_all(folder.join("routed"))?;
    let mut text = String::new();
    for list in &lists {
        text += &std::fs::read_to_string(list).with_context(|| list.display().to_string())?;
        text += "\n";
    }
    let corpus = Corpus {
        root: &root,
        folder: &folder,
        traxide: &traxide,
        freerouting: freerouting.as_deref(),
        kicad: Kicad::from_args(args),
        timeout,
    };
    let mut rows = Vec::new();
    for board in in_shard(boards(&text), shard) {
        let row = match Project::parse(board) {
            Some(project) => corpus.route_project(&project)?,
            None => corpus.route_fixture(board)?,
        };
        eprintln!(
            "{}",
            row.cells(row.author.is_some(), row.kicad_drc.is_some())
        );
        rows.push(row);
    }
    let record = Record {
        timeout_seconds: timeout.as_secs(),
        boards: rows
            .iter()
            .map(|row| (row.board.clone(), row.outcome()))
            .collect(),
    };
    let results = folder.join(format!("results-{}-of-{}.json", shard.0, shard.1));
    std::fs::write(&results, serde_json::to_string_pretty(&record)? + "\n")?;
    eprintln!("wrote {}", results.display());
    let table = table(&rows, freerouting.is_some());
    println!("{table}");
    if let Ok(summary) = std::env::var("GITHUB_STEP_SUMMARY") {
        std::fs::write(summary, format!("## Corpus\n\n{table}\n"))?;
    }
    Ok(())
}

/// What routing a board of the corpus takes.
struct Corpus<'a> {
    root: &'a Path,
    folder: &'a Path,
    traxide: &'a Path,
    freerouting: Option<&'a str>,
    kicad: Kicad,
    timeout: Duration,
}

impl Corpus<'_> {
    /// A Specctra design from Freerouting's repository, routed into sessions.
    fn route_fixture(&self, board: &str) -> Result<Row> {
        let file = self.folder.join(board.replace(['/', ' '], "_"));
        if !file.exists() {
            let url = format!("{SOURCE}{}", board.replace(' ', "%20").replace('+', "%2B"));
            download(&url, &file)?;
        }
        let name = file
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let session = |router: &str| {
            self.folder
                .join("routed")
                .join(format!("{name}.{router}.ses"))
        };
        let judge_session = |session: &Path| {
            judge(self.traxide, &file, Some(session)).map_err(|why| {
                quoting_line(&why, &std::fs::read_to_string(session).unwrap_or_default())
            })
        };
        let traxide_session = session("traxide");
        let traxide_run = traxide_route(self.traxide, &file, &traxide_session, self.timeout);
        let mut row = Row {
            board: name.clone(),
            before: judge(self.traxide, &file, None).ok(),
            author: None,
            traxide: attempt(traxide_run, &traxide_session, self.timeout, || {
                judge_session(&traxide_session)
            })?,
            freerouting: None,
            kicad_drc: None,
        };
        if let Some(command) = self.freerouting {
            let freerouting_session = session("freerouting");
            row.freerouting = Some(attempt(
                freerouting_run(command, &file, &freerouting_session)?,
                &freerouting_session,
                self.timeout,
                || judge_session(&freerouting_session),
            )?);
        }
        Ok(row)
    }

    /// A KiCad project at a commit, stripped of its author's routes: traxide
    /// routes the board, Freerouting the DSN that KiCad exports of it, and the
    /// board as committed is judged as its author routed it.
    fn route_project(&self, project: &Project) -> Result<Row> {
        let name = project.name();
        let folder = self
            .folder
            .join("projects")
            .join(format!("{name}-{}", project.commit));
        std::fs::create_dir_all(&folder)?;
        let committed = folder.join(format!("{}.kicad_pcb", project.stem()));
        if !committed.exists() {
            for extension in RULE_FILES {
                download_if_present(
                    &project.url(extension),
                    &committed.with_extension(extension),
                )?;
            }
            download(&project.url("kicad_pcb"), &committed)?;
        }
        let stripped = folder.join("stripped.kicad_pcb");
        std::fs::write(
            &stripped,
            strip_unlocked_copper(&std::fs::read_to_string(&committed)?),
        )?;
        for extension in RULE_FILES {
            let rules = committed.with_extension(extension);
            if rules.exists() {
                std::fs::copy(&rules, stripped.with_extension(extension))?;
            }
        }
        let routed = |router: &str| folder.join(format!("{router}.kicad_pcb"));
        let judge_board = |board: &Path| judge(self.traxide, board, None);
        let traxide_board = routed("traxide");
        let traxide_run = traxide_route(self.traxide, &stripped, &traxide_board, self.timeout);
        let mut row = Row {
            board: name,
            before: judge_board(&stripped).ok(),
            author: Some(judge_board(&committed).map_err(|why| anyhow!(why))?),
            traxide: attempt(traxide_run, &traxide_board, self.timeout, || {
                judge_board(&traxide_board)
            })?,
            freerouting: None,
            kicad_drc: None,
        };
        if let Some(command) = self.freerouting {
            row.freerouting = Some(self.freerouting_on_project(command, &folder)?);
        }
        row.kicad_drc = Some(self.kicad_drc(&folder, &committed, &row)?);
        Ok(row)
    }

    /// What KiCad's DRC, with the zones refilled, finds on each routing of a
    /// project's board: traxide's, Freerouting's and its author's. Each is
    /// judged as a copy with the board's rules beside it, as refilling saves
    /// the board.
    fn kicad_drc(&self, folder: &Path, committed: &Path, row: &Row) -> Result<String> {
        let with_rules = |from: &Path, stem: &str| -> Result<()> {
            for extension in RULE_FILES {
                let rules = from.with_extension(extension);
                if rules.exists() {
                    std::fs::copy(&rules, folder.join(stem).with_extension(extension))?;
                }
            }
            Ok(())
        };
        let mut verdicts = Vec::new();
        let mut judge = |router: &str, stem: &str| {
            let verdict = drc_verdict(&self.kicad.judge_board(self.root, folder, stem));
            verdicts.push(format!("{router}: {verdict}"));
        };
        if matches!(row.traxide, Attempt::Judged { .. }) {
            judge("traxide", "traxide");
        }
        if matches!(row.freerouting, Some(Attempt::Judged { .. })) {
            with_rules(&folder.join("stripped.kicad_pcb"), "freerouting")?;
            judge("Freerouting", "freerouting");
        }
        std::fs::copy(committed, folder.join("author.kicad_pcb"))?;
        with_rules(committed, "author")?;
        judge("author", "author");
        Ok(verdicts.join("; "))
    }

    /// Freerouting on the DSN that KiCad exports of `FOLDER/stripped.kicad_pcb`,
    /// its session imported into the board by KiCad, as a user would.
    fn freerouting_on_project(&self, command: &str, folder: &Path) -> Result<Attempt> {
        if let Err(why) = self.kicad.export_dsn(self.root, folder, "stripped") {
            return Ok(Attempt::Failed {
                why: format!("KiCad exported no DSN: {}", first_line(&why.to_string())),
            });
        }
        let session = folder.join("freerouting.ses");
        let routed = folder.join("freerouting.kicad_pcb");
        let run = freerouting_run(command, &folder.join("stripped.dsn"), &session)?;
        attempt(run, &session, self.timeout, || {
            self.kicad
                .import(
                    self.root,
                    folder,
                    "stripped.kicad_pcb",
                    "freerouting.ses",
                    "freerouting.kicad_pcb",
                )
                .map_err(|why| {
                    format!(
                        "KiCad imported no session: {}",
                        first_line(&why.to_string())
                    )
                })?;
            judge(self.traxide, &routed, None)
        })
    }
}

/// traxide routing `board` into `output`, negotiating for three quarters of
/// `timeout`: real boards such as TokenBoard, and congested ones that traxide
/// routes a second time (TRX-56), take longer than the corpus's limit, and a
/// board stopped at the limit tells the gate nothing, so traxide stops
/// negotiating early enough to finish within it, with what it routed by then.
fn traxide_route(traxide: &Path, board: &Path, output: &Path, timeout: Duration) -> Command {
    let mut run = Command::new(traxide);
    run.arg("route")
        .arg("--quiet")
        .arg(board)
        .arg("-o")
        .arg(output)
        .arg("--time-limit")
        .arg(negotiation_limit(timeout).as_secs().to_string());
    run
}

/// How long traxide negotiates on a board of the corpus.
pub fn negotiation_limit(timeout: Duration) -> Duration {
    timeout * 3 / 4
}

fn freerouting_run(command: &str, design: &Path, session: &Path) -> Result<Command> {
    let mut words = command.split_whitespace();
    let mut run = Command::new(words.next().context("--freerouting names no command")?);
    run.args(words)
        .arg("-de")
        .arg(design)
        .arg("-do")
        .arg(session)
        .arg("--gui.enabled=false");
    Ok(run)
}

fn boards(list: &str) -> impl Iterator<Item = &str> {
    list.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

/// "K/N": the K-th of N shards, counting from 1.
fn shard(text: &str) -> Result<(usize, usize)> {
    let (k, n) = text.split_once('/').context("--shard takes K/N")?;
    let (k, n) = (k.parse()?, n.parse()?);
    ensure!(
        (1..=n).contains(&k),
        "--shard {text}: K must be between 1 and N"
    );
    Ok((k, n))
}

/// Every N-th board, starting at the K-th: shards of about equal size.
fn in_shard<'a>(
    boards: impl Iterator<Item = &'a str>,
    (k, n): (usize, usize),
) -> impl Iterator<Item = &'a str> {
    boards
        .enumerate()
        .filter(move |(i, _)| i % n == k - 1)
        .map(|(_, b)| b)
}

fn download(url: &str, file: &Path) -> Result<()> {
    ensure!(
        download_if_present(url, file)?,
        "downloading {url} failed: not found"
    );
    Ok(())
}

/// Downloads `url` into `file`; false when there is no such file.
fn download_if_present(url: &str, file: &Path) -> Result<bool> {
    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--location",
            "--write-out",
            "%{http_code}",
            "--output",
        ])
        .arg(file)
        .arg(url)
        .output()
        .context("curl")?;
    let status = String::from_utf8_lossy(&output.stdout);
    if status == "404" {
        let _ = std::fs::remove_file(file);
        return Ok(false);
    }
    ensure!(
        output.status.success() && status == "200",
        "downloading {url} failed: {status} {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(true)
}

struct Row {
    board: String,
    /// The board as it came, before either router.
    before: Option<Judgement>,
    /// A board committed routed, as its author routed it.
    author: Option<Judgement>,
    traxide: Attempt,
    freerouting: Option<Attempt>,
    /// What KiCad's DRC finds on a KiCad project's routings.
    kicad_drc: Option<String>,
}

/// KiCad's errors by kind and its missing connections, or why it judged nothing.
fn drc_verdict(judged: &Result<kicad::Judgement>) -> String {
    let judgement = match judged {
        Ok(judgement) => judgement,
        Err(why) => return format!("not judged: {}", first_line(&why.to_string())),
    };
    if judgement.is_clean() {
        return "clean".into();
    }
    let kinds: Vec<String> = judgement
        .violation_kinds
        .iter()
        .map(|(kind, n)| format!("{kind} {n}"))
        .collect();
    let errors = match judgement.violations.len() {
        0 => "no errors".to_owned(),
        n => format!("{n} errors ({})", kinds.join(", ")),
    };
    format!("{errors}, {} unconnected", judgement.unconnected.len())
}

/// What `traxide check --json` says about a board and its routes.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Judgement {
    nets: usize,
    incomplete_nets: usize,
    violations: usize,
}

#[derive(Debug)]
enum Attempt {
    Judged { seconds: f64, judgement: Judgement },
    Stopped { seconds: f64 },
    Failed { why: String },
}

/// Runs a router and, once it wrote `output`, judges what it wrote.
fn attempt(
    mut router: Command,
    output: &Path,
    timeout: Duration,
    judge_output: impl FnOnce() -> Result<Judgement, String>,
) -> Result<Attempt> {
    // An output left from an earlier run must not pass for this one's.
    let _ = std::fs::remove_file(output);
    let started = Instant::now();
    let mut child = router
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let finished = loop {
        if child.try_wait()?.is_some() {
            break true;
        }
        if started.elapsed() > timeout {
            child.kill()?;
            break false;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let seconds = started.elapsed().as_secs_f64();
    let stderr = child.wait_with_output()?.stderr;
    Ok(if !finished {
        Attempt::Stopped { seconds }
    } else if !output.exists() {
        Attempt::Failed {
            why: first_line(&String::from_utf8_lossy(&stderr)),
        }
    } else {
        match judge_output() {
            Ok(judgement) => Attempt::Judged { seconds, judgement },
            Err(why) => Attempt::Failed { why },
        }
    })
}

fn judge(traxide: &Path, board: &Path, session: Option<&Path>) -> Result<Judgement, String> {
    let mut check = Command::new(traxide);
    check.arg("check").arg(board).args(session).arg("--json");
    let output = check.output().map_err(|e| e.to_string())?;
    judgement(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| first_line(&String::from_utf8_lossy(&output.stderr)))
}

/// Reads the report of `traxide check --json`.
fn judgement(json: &str) -> Option<Judgement> {
    let report: serde_json::Value = serde_json::from_str(json).ok()?;
    let count = |key: &str| report[key].as_u64().and_then(|n| usize::try_from(n).ok());
    Some(Judgement {
        nets: count("nets")?,
        incomplete_nets: count("incomplete_nets")?,
        violations: report["violations"]
            .as_object()?
            .values()
            .filter_map(serde_json::Value::as_u64)
            .map(|n| usize::try_from(n).unwrap_or(usize::MAX))
            .sum(),
    })
}

/// `why`, a reason that names a line of the session, with that line, so that
/// the table shows what the router wrote there.
fn quoting_line(why: &str, session: &str) -> String {
    let number = why
        .strip_prefix("line ")
        .and_then(|rest| rest.split_once(':'))
        .and_then(|(number, _)| number.parse::<usize>().ok());
    match number.and_then(|n| session.lines().nth(n.checked_sub(1)?)) {
        Some(line) => {
            let line: String = line.trim().chars().take(80).collect();
            format!("{why}, at `{}`", line.replace('|', "/"))
        }
        None => why.to_owned(),
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or("")
        .trim_start_matches("error: ")
        .replace('|', "/")
}

impl Row {
    /// traxide's attempt, as the gate compares it.
    fn outcome(&self) -> Outcome {
        let tenths = |seconds: f64| (seconds * 10.0).round() / 10.0;
        match &self.traxide {
            Attempt::Judged { seconds, judgement } => Outcome::Finished {
                seconds: tenths(*seconds),
                nets: judgement.nets,
                routed: judgement.nets - judgement.incomplete_nets,
                rules_broken: judgement
                    .violations
                    .saturating_sub(self.before.map_or(0, |b| b.violations)),
            },
            Attempt::Stopped { seconds } => Outcome::Stopped {
                seconds: tenths(*seconds),
            },
            Attempt::Failed { why } => Outcome::Failed { why: why.clone() },
        }
    }

    /// The row's cells, with one for the author's routing when the table
    /// has that column.
    fn cells(&self, with_author: bool, with_kicad: bool) -> String {
        let mut cells = vec![self.board.clone(), self.describe(&self.traxide)];
        if let Some(freerouting) = &self.freerouting {
            cells.push(self.describe(freerouting));
        }
        if with_author {
            cells.push(
                self.author
                    .map_or_else(|| "—".to_owned(), |author| self.result(author)),
            );
        }
        if with_kicad {
            cells.push(self.kicad_drc.clone().unwrap_or_else(|| "—".to_owned()));
        }
        format!("| {} |", cells.join(" | "))
    }

    /// Time, complete nets, and the rules broken beyond those the board broke.
    fn describe(&self, attempt: &Attempt) -> String {
        match attempt {
            Attempt::Judged { seconds, judgement } => {
                format!("{seconds:.1} s, {}", self.result(*judgement))
            }
            Attempt::Stopped { seconds } => format!("stopped after {seconds:.0} s"),
            Attempt::Failed { why } => format!("failed: {why}"),
        }
    }

    fn result(&self, judgement: Judgement) -> String {
        let broken_before = self.before.map_or(0, |b| b.violations);
        format!(
            "{} of {} nets, {} rules broken",
            judgement.nets - judgement.incomplete_nets,
            judgement.nets,
            judgement.violations.saturating_sub(broken_before)
        )
    }
}

fn table(rows: &[Row], with_freerouting: bool) -> String {
    let with_author = rows.iter().any(|row| row.author.is_some());
    let with_kicad = rows.iter().any(|row| row.kicad_drc.is_some());
    let mut columns = vec!["Board", "traxide"];
    if with_freerouting {
        columns.push("Freerouting");
    }
    if with_author {
        columns.push("Routed by its author");
    }
    if with_kicad {
        columns.push("KiCad's DRC");
    }
    let mut lines = vec![
        format!("| {} |", columns.join(" | ")),
        format!("|{}", " --- |".repeat(columns.len())),
    ];
    lines.extend(rows.iter().map(|row| row.cells(with_author, with_kicad)));
    lines.push(String::new());
    lines.push(totals("traxide", rows.iter().map(|r| (r, &r.traxide))));
    if with_freerouting {
        lines.push(totals(
            "Freerouting",
            rows.iter()
                .filter_map(|r| r.freerouting.as_ref().map(|f| (r, f))),
        ));
    }
    lines.join("\n")
}

fn totals<'a>(router: &str, attempts: impl Iterator<Item = (&'a Row, &'a Attempt)>) -> String {
    let (mut complete, mut incomplete, mut broke, mut stopped, mut failed, mut seconds) =
        (0, 0, 0, 0, 0, 0.0);
    for (row, attempt) in attempts {
        match attempt {
            Attempt::Judged {
                seconds: s,
                judgement,
            } => {
                seconds += s;
                if judgement.violations > row.before.map_or(0, |b| b.violations) {
                    broke += 1;
                } else if judgement.incomplete_nets == 0 {
                    complete += 1;
                } else {
                    incomplete += 1;
                }
            }
            Attempt::Stopped { seconds: s } => {
                seconds += s;
                stopped += 1;
            }
            Attempt::Failed { .. } => failed += 1,
        }
    }
    format!(
        "{router}: {complete} complete, {incomplete} incomplete, {broke} breaking rules, {stopped} stopped, {failed} failed; {seconds:.0} s in all."
    )
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_report_of_traxide_check() {
        let json = r#"{"nets":31,"incomplete_nets":2,"unconnected_pads":3,
            "violations":{"clearance":4,"edge":1},"tracks":9,"vias":1,"track_length_mm":3.5}"#;
        assert_eq!(
            judgement(json),
            Some(Judgement {
                nets: 31,
                incomplete_nets: 2,
                violations: 5
            })
        );
    }

    #[test]
    fn an_error_at_a_line_quotes_that_line() {
        let session = "(session s\n  (routes\n    (via Round1 1 2)\n";
        assert_eq!(
            quoting_line("line 3: unknown via padstack Round1", session),
            "line 3: unknown via padstack Round1, at `(via Round1 1 2)`"
        );
        assert_eq!(quoting_line("no line", session), "no line");
    }

    #[test]
    fn an_error_is_no_report() {
        assert_eq!(judgement(""), None);
    }

    #[test]
    fn shards_take_every_nth_board() {
        let boards = ["a", "b", "c", "d", "e"];
        let second: Vec<&str> = in_shard(boards.into_iter(), (2, 2)).collect();
        assert_eq!(second, ["b", "d"]);
        assert!(shard("3/2").is_err());
    }

    #[test]
    fn rules_the_board_broke_already_are_not_counted_against_a_router() {
        let row = Row {
            board: "b".into(),
            before: Some(Judgement {
                nets: 4,
                incomplete_nets: 4,
                violations: 2,
            }),
            traxide: Attempt::Judged {
                seconds: 1.0,
                judgement: Judgement {
                    nets: 4,
                    incomplete_nets: 0,
                    violations: 2,
                },
            },
            freerouting: None,
            author: None,
            kicad_drc: None,
        };
        assert_eq!(
            row.cells(false, false),
            "| b | 1.0 s, 4 of 4 nets, 0 rules broken |"
        );
        assert_eq!(
            row.outcome(),
            Outcome::Finished {
                seconds: 1.0,
                nets: 4,
                routed: 4,
                rules_broken: 0
            }
        );
    }

    #[test]
    fn traxide_negotiates_on_every_board_for_three_quarters_of_the_limit() {
        let run = traxide_route(
            Path::new("traxide"),
            Path::new("board.dsn"),
            Path::new("board.ses"),
            Duration::from_secs(120),
        );
        let args: Vec<_> = run.get_args().collect();
        assert_eq!(
            args,
            [
                "route",
                "--quiet",
                "board.dsn",
                "-o",
                "board.ses",
                "--time-limit",
                "90"
            ]
        );
    }

    #[test]
    fn a_board_committed_routed_shows_its_authors_routing_beside_the_routers() {
        let judged = |incomplete_nets| Judgement {
            nets: 9,
            incomplete_nets,
            violations: 1,
        };
        let row = |board: &str, author| Row {
            board: board.into(),
            before: Some(judged(9)),
            author,
            traxide: Attempt::Judged {
                seconds: 2.0,
                judgement: judged(0),
            },
            freerouting: Some(Attempt::Failed { why: "no".into() }),
            kicad_drc: None,
        };
        let table = table(
            &[row("tokenboard-kb", Some(judged(1))), row("fixture", None)],
            true,
        );
        let lines: Vec<&str> = table.lines().take(4).collect();
        assert_eq!(
            lines,
            [
                "| Board | traxide | Freerouting | Routed by its author |",
                "| --- | --- | --- | --- |",
                "| tokenboard-kb | 2.0 s, 9 of 9 nets, 0 rules broken | failed: no | 8 of 9 nets, 0 rules broken |",
                "| fixture | 2.0 s, 9 of 9 nets, 0 rules broken | failed: no | — |",
            ]
        );
    }

    fn drc(kinds: &[(&str, usize)], unconnected: usize) -> kicad::Judgement {
        kicad::Judgement {
            violations: kinds
                .iter()
                .flat_map(|&(kind, n)| std::iter::repeat_n(kind.to_owned(), n))
                .collect(),
            violation_kinds: kinds.iter().map(|&(k, n)| (k.to_owned(), n)).collect(),
            unconnected: vec!["pad".into(); unconnected],
            tracks: 0,
            vias: 0,
            length_mm: 0.0,
            zones: 0,
            filled_zones: 0,
        }
    }

    #[test]
    fn kicads_drc_names_its_errors_by_kind_and_its_missing_connections() {
        assert_eq!(drc_verdict(&Ok(drc(&[], 0))), "clean");
        assert_eq!(
            drc_verdict(&Ok(drc(
                &[("clearance", 2), ("copper_edge_clearance", 1)],
                1
            ))),
            "3 errors (clearance 2, copper_edge_clearance 1), 1 unconnected"
        );
        assert_eq!(
            drc_verdict(&Err(anyhow!("KiCad failed: no docker\nmore"))),
            "not judged: KiCad failed: no docker"
        );
    }

    #[test]
    fn a_kicad_project_shows_what_kicads_drc_finds_beside_traxides_check() {
        let judged = Judgement {
            nets: 9,
            incomplete_nets: 0,
            violations: 0,
        };
        let row = |board: &str, kicad_drc: Option<&str>| Row {
            board: board.into(),
            before: Some(judged),
            author: Some(judged),
            traxide: Attempt::Judged {
                seconds: 2.0,
                judgement: judged,
            },
            freerouting: None,
            kicad_drc: kicad_drc.map(str::to_owned),
        };
        let table = table(
            &[
                row("tokenboard-kb", Some("traxide: clean; author: clean")),
                row("fixture", None),
            ],
            false,
        );
        let lines: Vec<&str> = table.lines().take(4).collect();
        assert_eq!(
            lines,
            [
                "| Board | traxide | Routed by its author | KiCad's DRC |",
                "| --- | --- | --- | --- |",
                "| tokenboard-kb | 2.0 s, 9 of 9 nets, 0 rules broken | 9 of 9 nets, 0 rules broken | traxide: clean; author: clean |",
                "| fixture | 2.0 s, 9 of 9 nets, 0 rules broken | 9 of 9 nets, 0 rules broken | — |",
            ]
        );
    }
}
