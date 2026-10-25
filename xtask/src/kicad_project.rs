//! KiCad projects in the corpus: a board in a GitHub repository at a fixed
//! commit, with the project and custom rules of the same name beside it.

/// One line of a corpus list: `https://github.com/OWNER/REPO COMMIT PATH.kicad_pcb`.
#[derive(Debug, PartialEq)]
pub struct Project<'a> {
    repository: &'a str,
    pub commit: &'a str,
    board: &'a str,
}

const GITHUB: &str = "https://github.com/";
const RAW_GITHUB: &str = "https://raw.githubusercontent.com/";

/// The files of a KiCad project that traxide reads, besides the board.
pub const RULE_FILES: [&str; 2] = ["kicad_pro", "kicad_dru"];

impl<'a> Project<'a> {
    /// The project a corpus line names, or `None` for a line that names
    /// something else, such as a fixture of Freerouting's.
    pub fn parse(line: &'a str) -> Option<Self> {
        let mut words = line.split_whitespace();
        let repository = words.next()?.strip_prefix(GITHUB)?.trim_end_matches('/');
        let (commit, board) = (words.next()?, words.next()?);
        (words.next().is_none() && board.ends_with(".kicad_pcb")).then_some(Self {
            repository,
            commit,
            board,
        })
    }

    /// The repository's name and the board's: `tokenboard-kb`.
    pub fn name(&self) -> String {
        let repository = self.repository.rsplit('/').next().unwrap_or_default();
        format!("{repository}-{}", self.stem())
    }

    /// The board's file name without its extension: `kb`.
    pub fn stem(&self) -> &'a str {
        let file = self.board.rsplit('/').next().unwrap_or_default();
        file.trim_end_matches(".kicad_pcb")
    }

    /// Where the board's file with `extension` lies at the pinned commit.
    pub fn url(&self, extension: &str) -> String {
        let path = self.board.trim_end_matches("kicad_pcb");
        format!(
            "{RAW_GITHUB}{}/{}/{path}{extension}",
            self.repository, self.commit
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "https://github.com/h3ll5ur7er/tokenboard 478b4f9 kb/kb.kicad_pcb";

    #[test]
    fn a_line_names_a_board_in_a_repository_at_a_commit() {
        let project = Project::parse(LINE).unwrap();
        assert_eq!(project.name(), "tokenboard-kb");
        assert_eq!(project.commit, "478b4f9");
        assert_eq!(
            project.url("kicad_dru"),
            "https://raw.githubusercontent.com/h3ll5ur7er/tokenboard/478b4f9/kb/kb.kicad_dru"
        );
    }

    #[test]
    fn a_fixture_of_freeroutings_is_no_project() {
        assert_eq!(Project::parse("fixtures/Issue015-StackOverflow.dsn"), None);
        assert_eq!(
            Project::parse("https://github.com/o/r 478b4f9 board.dsn"),
            None
        );
    }
}
