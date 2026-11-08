//! The only lines of agent documents the daemon writes (D11): task progress
//! in `tasks.md` (`- status:`, `- evidence:`) and commit progress in
//! `commits.md` (`- status:`, `- sha:`). Hashes ignore the task ones
//! ([`crate::tasks::without_progress`]). Everything else stays byte for byte.

use xherd_core::domain::TaskStatus;

/// The `C01` of a commit entry heading (`## C01: subject`).
fn commit_heading(line: &str) -> Option<String> {
    let rest = line.trim_start_matches('#');
    let level = line.len() - rest.len();
    if !(2..=4).contains(&level) || !rest.starts_with(' ') {
        return None;
    }
    let digits = rest.trim_start().strip_prefix('C')?;
    let n = digits.chars().take_while(char::is_ascii_digit).count();
    (n > 0).then(|| format!("C{}", &digits[..n]))
}

/// The line ending of `line` (`"\r\n"`, `"\n"` or empty).
fn eol_of(line: &str) -> &str {
    &line[line.trim_end_matches(['\n', '\r']).len()..]
}

/// Set each `(key, value)` field in the section of heading `id`: an existing
/// `- key:` line is replaced (indent and bullet kept), a missing one is added
/// below the previous field (or the heading). `None` when the section is not
/// found. An empty value leaves `- key:` empty.
fn set_fields(
    text: &str,
    heading: fn(&str) -> Option<String>,
    id: &str,
    fields: &[(&str, &str)],
) -> Option<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let start = lines
        .iter()
        .position(|l| heading(l.trim_end_matches(['\n', '\r'])).is_some_and(|h| h == id))?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with('#'))
        .map_or(lines.len(), |n| start + 1 + n);
    let eol = match eol_of(lines[start]) {
        "" => "\n",
        e => e,
    };
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    let mut last_field = start;
    for (key, value) in fields {
        let line_for = |prefix: &str, eol: &str| {
            let sep = if value.is_empty() { "" } else { " " };
            format!("{prefix}{key}:{sep}{value}{eol}")
        };
        let found = (start + 1..end).find(|&i| {
            let item = lines[i].trim_start();
            item.strip_prefix("- ")
                .or_else(|| item.strip_prefix("* "))
                .and_then(|r| r.split_once(':'))
                .is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(key))
        });
        match found {
            Some(i) => {
                let item = lines[i].trim_start();
                let indent_and_bullet = &lines[i][..lines[i].len() - item.len() + 2];
                out[i] = line_for(indent_and_bullet, eol_of(lines[i]));
                last_field = i;
            }
            None => {
                let at = &mut out[last_field];
                if !at.ends_with('\n') {
                    at.push_str(eol);
                }
                at.push_str(&line_for("- ", eol));
            }
        }
    }
    Some(out.concat())
}

/// `text` with task `id`'s `- status:` (and `- evidence:` when given, on one
/// line) set. `None` when the section is not found.
pub fn set_task(
    text: &str,
    id: &str,
    status: TaskStatus,
    evidence: Option<&str>,
) -> Option<String> {
    let status = match status {
        TaskStatus::Todo => "todo",
        TaskStatus::Doing => "doing",
        TaskStatus::Done => "done",
        TaskStatus::Blocked => "blocked",
    };
    let evidence = evidence.map(|e| {
        e.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" | ")
    });
    let mut fields = vec![("status", status)];
    if let Some(e) = &evidence {
        fields.push(("evidence", e));
    }
    set_fields(text, crate::tasks::heading_id, id, &fields)
}

/// `text` with commit `id`'s `- status:` and `- sha:` set (an empty sha
/// leaves the line empty). `None` when the entry is not found.
pub fn set_commit(text: &str, id: &str, status: &str, sha: &str) -> Option<String> {
    set_fields(
        text,
        commit_heading,
        id,
        &[("status", status), ("sha", sha)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASKS: &str = "# Tasks\n\n## T01: a\n- status: todo\n- files: `a`\n- evidence:\n\n## T02: b\n- files: b\n\n## Assumptions\n- status: x\n";

    #[test]
    fn replaces_existing_lines_only_in_the_section() {
        let out = set_task(TASKS, "T01", TaskStatus::Done, Some("ran\nok")).unwrap();
        assert_eq!(
            out,
            TASKS
                .replace("- status: todo", "- status: done")
                .replace("- evidence:\n", "- evidence: ran | ok\n")
        );
        // Without evidence the evidence line is left alone.
        let out = set_task(TASKS, "T01", TaskStatus::Doing, None).unwrap();
        assert_eq!(out, TASKS.replace("- status: todo", "- status: doing"));
    }

    #[test]
    fn adds_missing_lines_after_the_heading_or_last_field() {
        let out = set_task(TASKS, "T02", TaskStatus::Done, Some("ok")).unwrap();
        assert!(
            out.contains("## T02: b\n- status: done\n- evidence: ok\n- files: b\n"),
            "{out}"
        );
        // Everything outside the progress lines is untouched.
        assert_eq!(
            crate::tasks::without_progress(&out),
            crate::tasks::without_progress(TASKS)
        );
        let text = "## T01: a\n- files: x\n- status: todo";
        assert_eq!(
            set_task(text, "T01", TaskStatus::Blocked, None).unwrap(),
            "## T01: a\n- files: x\n- status: blocked"
        );
        // Evidence added below an existing status line.
        let text = "## T01: a\n- status: doing\n- files: x\n";
        assert_eq!(
            set_task(text, "T01", TaskStatus::Done, Some("e")).unwrap(),
            "## T01: a\n- status: done\n- evidence: e\n- files: x\n"
        );
    }

    #[test]
    fn keeps_crlf_and_indentation() {
        let text = "## T01: a\r\n  * status: todo\r\n- files: x\r\n";
        assert_eq!(
            set_task(text, "T01", TaskStatus::Doing, None).unwrap(),
            "## T01: a\r\n  * status: doing\r\n- files: x\r\n"
        );
        let text = "## T01: a\r\n- files: x\r\n";
        assert_eq!(
            set_task(text, "T01", TaskStatus::Doing, Some("e")).unwrap(),
            "## T01: a\r\n- status: doing\r\n- evidence: e\r\n- files: x\r\n"
        );
    }

    #[test]
    fn section_not_found() {
        assert!(set_task(TASKS, "T09", TaskStatus::Done, None).is_none());
        assert!(set_task("no tasks here\n", "T01", TaskStatus::Done, None).is_none());
        assert!(set_commit("## T01: a\n", "C01", "done", "abc").is_none());
    }

    #[test]
    fn commit_entries() {
        let text = "## C01: first\n- tasks: T01\n- files: `a`\n- status: proposed      # proposed, done\n- sha:\n\n## C02: second\n- tasks: T02\n";
        let out = set_commit(text, "C01", "done", "3f2a9c1").unwrap();
        assert_eq!(
            out,
            text.replace("- status: proposed      # proposed, done", "- status: done")
                .replace("- sha:\n", "- sha: 3f2a9c1\n")
        );
        let out = set_commit(text, "C02", "proposed", "").unwrap();
        assert!(
            out.ends_with("## C02: second\n- status: proposed\n- sha:\n- tasks: T02\n"),
            "{out}"
        );
    }
}
