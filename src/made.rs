//! What a session made, as one list (#137).
//!
//! The bottom line of a cell is where the work comes out. #134 made it the
//! pull request line and gave it a list to drop when there was more than one
//! number to show; a published page is the same kind of thing — something the
//! session produced and somebody will want to open — so it goes on the same
//! line and into the same list rather than fighting for a row of its own in a
//! cell that has no room for a fourth.
//!
//! Pull requests come first, newest number first, and the pages after them: a
//! pull request is the work and a page is the report about it, so that is the
//! order somebody reads them in.

use crate::logos::Mark;
use crate::model::SessionInfo;
use crate::pr;

/// One line of the list a cell drops.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub mark: Mark,
    /// What goes inside the node's body: a pull request's number, a page's
    /// version. A page whose line never counted its publishes has nothing to
    /// put here and wears its mark alone.
    pub chip: String,
    /// What it is about: a pull request's title, a page's own name. Empty
    /// where nobody could say (#112).
    pub detail: String,
    /// What the status bar calls it, and what kind of thing it is, when it
    /// opens (#57).
    pub name: String,
    pub kind: &'static str,
    pub url: String,
    /// How the board would draw a pull request (#78). `None` for a page, and
    /// for a number nobody could speak for: neither has a state to draw.
    pub look: Option<pr::Look>,
    pub running: bool,
    pub signed_off: bool,
}

pub const PULL_REQUEST: &str = "pull request";
pub const ARTIFACT: &str = "artifact";

/// Everything a session has to show on that line.
pub fn rows(info: &SessionInfo) -> Vec<Row> {
    let prs = pr::rows(&info.prs, &info.prs_unanswered).into_iter().map(|row| Row {
        mark: Mark::GitHub,
        chip: row.label.clone(),
        detail: row.title,
        name: row.label,
        kind: PULL_REQUEST,
        url: row.url,
        look: row.look,
        running: row.running,
        signed_off: row.signed_off,
    });
    // Newest last on the cell, newest first here: a list is read from the top
    // and the pull requests above it are already newest first.
    let pages = info.artifacts.iter().rev().map(|a| Row {
        mark: Mark::Artifact,
        chip: a.version_label().unwrap_or_default(),
        detail: a.label(),
        name: a.label(),
        kind: ARTIFACT,
        url: a.url(),
        look: None,
        running: false,
        signed_off: false,
    });
    prs.chain(pages).collect()
}

/// What that line does when it is pressed (#134).
#[derive(Debug, PartialEq)]
pub enum Press {
    /// Nothing on it: the press belongs to the cell as a whole.
    Cell,
    /// One thing, opened where it lives.
    One { name: String, kind: &'static str, url: Option<String> },
    /// Several, so the list drops down instead of one of them opening.
    List,
}

/// The line draws the newest thing a session made and can open it; what it
/// cannot do is say that there are others, which is the whole reason the list
/// exists (#134). A number GitHub would not answer for counts as one of them:
/// the cell falls back to drawing it, so the line is already speaking for it.
pub fn press(info: &SessionInfo) -> Press {
    match rows(info).as_slice() {
        [] => Press::Cell,
        [one] => Press::One {
            name: one.name.clone(),
            kind: one.kind,
            // The sandbox's pull requests and pages are made up, so they go
            // nowhere, the same way its sessions have no terminal to jump to.
            url: (!info.synthetic).then(|| one.url.clone()),
        },
        _ => Press::List,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Artifact;
    use crate::model::Phase;

    fn session() -> SessionInfo {
        SessionInfo::synthetic(1, "SESS", Phase::Working)
    }

    fn page(id: &str, title: &str) -> Artifact {
        Artifact { id: id.into(), title: title.into(), version: Some(2) }
    }

    /// A page is one of the things the line speaks for, so a session with a
    /// pull request and a page has two things on one line and the list is
    /// what shows both (#137).
    #[test]
    fn a_page_shares_the_line_with_the_pull_request() {
        let mut info = session();
        info.prs_unanswered = vec![crate::pr::PrRef { repo: "o/r".into(), number: 7 }];
        assert_eq!(press(&info), Press::One { name: "#7".into(), kind: PULL_REQUEST, url: None });
        info.artifacts = vec![page("A1", "report")];
        assert_eq!(press(&info), Press::List, "two things on one line is what the list is for");
        let rows = rows(&info);
        assert_eq!(rows.iter().map(|r| r.mark).collect::<Vec<_>>(), vec![Mark::GitHub, Mark::Artifact]);
        assert_eq!(rows[1].name, "report");
        assert_eq!(rows[1].url, "https://claude.ai/artifact/A1");
    }

    /// A session that made only pages still has a line to press, and one page
    /// opens straight away rather than dropping a list of one.
    #[test]
    fn a_session_with_only_pages_opens_the_newest() {
        let mut info = session();
        info.synthetic = false;
        assert_eq!(press(&info), Press::Cell);
        info.artifacts = vec![page("A1", "first")];
        assert_eq!(
            press(&info),
            Press::One {
                name: "first".into(),
                kind: ARTIFACT,
                url: Some("https://claude.ai/artifact/A1".into())
            }
        );
        info.artifacts.push(page("A2", "second"));
        assert_eq!(press(&info), Press::List);
        // Newest first in a list that is read downwards.
        assert_eq!(rows(&info).iter().map(|r| r.name.clone()).collect::<Vec<_>>(), vec!["second", "first"]);
    }

    /// A cell's output line opens what it is showing, and drops a list when
    /// it is showing one of several (#134).
    #[test]
    fn a_cell_with_more_than_one_pull_request_drops_its_list() {
        let mut info = session();
        info.synthetic = false;
        assert_eq!(press(&info), Press::Cell, "an empty line belongs to the cell");

        info.prs = vec![pr::Pr::synthetic(11, pr::Look::Open)];
        assert_eq!(
            press(&info),
            Press::One {
                name: "#11".into(),
                kind: PULL_REQUEST,
                url: Some("https://github.com/sandbox/demo/pull/11".into()),
            }
        );

        // A number GitHub would not answer for is one of them: the cell falls
        // back to drawing it, so the line is already speaking for it (#112).
        info.prs_unanswered = vec![pr::PrRef { repo: "sandbox/demo".into(), number: 12 }];
        assert_eq!(press(&info), Press::List);

        info.prs_unanswered.clear();
        info.prs.push(pr::Pr::synthetic(12, pr::Look::Merged));
        assert_eq!(press(&info), Press::List);

        // The sandbox has nowhere to send a click, the way its sessions have
        // no terminal to jump to.
        info.prs.pop();
        info.synthetic = true;
        assert_eq!(press(&info), Press::One { name: "#11".into(), kind: PULL_REQUEST, url: None });
    }
}
