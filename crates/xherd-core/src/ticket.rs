//! The summary xherd offers to post on an agent's ticket (R-308). It is a
//! draft: the user reads it, edits it and decides; nothing is posted without
//! that. It is built from what the daemon already knows (finished tasks,
//! committed work, the pull request), so it needs no model and carries no
//! text from the ticket itself.

use crate::agent::Agent;
use crate::domain::{CommitStatus, TaskStatus};

/// Most lines of tasks and commits each; the rest is counted.
const MAX_LINES: usize = 8;
/// A line longer than this is cut (titles are agent-written).
const MAX_LINE: usize = 140;

fn one_line(s: &str) -> String {
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > MAX_LINE {
        format!("{}…", s.chars().take(MAX_LINE - 1).collect::<String>())
    } else {
        s
    }
}

fn list(title: &str, items: Vec<String>) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let more = items.len().saturating_sub(MAX_LINES);
    let mut out = format!("{title}\n");
    for i in items.iter().take(MAX_LINES) {
        out.push_str(&format!("- {i}\n"));
    }
    if more > 0 {
        out.push_str(&format!("- … and {more} more\n"));
    }
    Some(out)
}

/// A short summary of what the agent has done on its ticket.
pub fn summary(a: &Agent) -> String {
    let done: Vec<String> = a
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Done)
        .map(|t| format!("{} ({})", one_line(&t.title), t.id))
        .collect();
    let commits: Vec<String> = a
        .commits
        .iter()
        .filter(|c| c.status == CommitStatus::Done)
        .map(|c| match &c.sha {
            Some(sha) => format!(
                "{} ({})",
                one_line(&c.subject),
                sha.chars().take(7).collect::<String>()
            ),
            None => one_line(&c.subject),
        })
        .collect();
    let mut parts = vec![format!("Update from the coding agent {}:\n", a.name)];
    parts.extend(list("Done:", done));
    parts.extend(list("Commits:", commits));
    if let Some(url) = &a.pr_url {
        let state = a
            .pr_state
            .map(|s| format!(" ({})", format!("{s:?}").to_lowercase()))
            .unwrap_or_default();
        parts.push(format!("Pull request: {url}{state}\n"));
    }
    if a.review_rounds > 0 {
        parts.push(format!("Review rounds: {}\n", a.review_rounds));
    }
    if parts.len() == 1 {
        parts.push(format!("The work is on the branch {}.\n", a.name));
    }
    parts.join("\n").trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Commit, Task};

    fn task(id: &str, title: &str, status: TaskStatus) -> Task {
        serde_json::from_value(serde_json::json!({"id": id, "title": title, "status": status}))
            .unwrap()
    }
    fn commit(id: &str, subject: &str, status: CommitStatus, sha: Option<&str>) -> Commit {
        serde_json::from_value(
            serde_json::json!({"id": id, "subject": subject, "files": [], "status": status, "sha": sha}),
        )
        .unwrap()
    }

    #[test]
    fn a_summary_lists_finished_work_only() {
        let mut a = Agent::new("PAY-1", "PAY-1", 0);
        a.tasks = vec![
            task("T01", "Add the retry", TaskStatus::Done),
            task("T02", "Still open", TaskStatus::Doing),
        ];
        a.commits = vec![
            commit(
                "C01",
                "Retry payments once",
                CommitStatus::Done,
                Some("abcdef1234567"),
            ),
            commit("C02", "Not yet", CommitStatus::Proposed, None),
        ];
        a.pr_url = Some("https://github.com/a/b/pull/7".into());
        a.review_rounds = 2;
        let s = summary(&a);
        assert!(
            s.starts_with("Update from the coding agent PAY-1:\n\nDone:\n- Add the retry (T01)\n"),
            "{s}"
        );
        assert!(
            s.contains("Commits:\n- Retry payments once (abcdef1)\n"),
            "{s}"
        );
        assert!(
            s.contains("Pull request: https://github.com/a/b/pull/7"),
            "{s}"
        );
        assert!(s.ends_with("Review rounds: 2"), "{s}");
        assert!(!s.contains("Still open") && !s.contains("Not yet"), "{s}");
    }

    #[test]
    fn long_lists_and_lines_are_cut_and_an_empty_agent_still_says_something() {
        let mut a = Agent::new("K-1", "K-1", 0);
        a.tasks = (0..11)
            .map(|i| task(&format!("T{i:02}"), &"x".repeat(300), TaskStatus::Done))
            .collect();
        let s = summary(&a);
        assert_eq!(s.matches("\n- ").count(), 9, "{s}");
        assert!(s.contains("- … and 3 more"), "{s}");
        assert!(s.lines().all(|l| l.chars().count() < 170), "{s}");

        let empty = summary(&Agent::new("K-2", "K-2", 0));
        assert_eq!(
            empty,
            "Update from the coding agent K-2:\n\nThe work is on the branch K-2."
        );
    }
}
