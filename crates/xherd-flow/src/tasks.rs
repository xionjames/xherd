//! xhSpec `tasks.md` (spec §7.3). Each task is a heading with a stable id and
//! a bullet list of fields:
//!
//! ```markdown
//! ## T01: Add the store
//! - status: todo
//! - files: `src/store.rs`, `src/lib.rs`
//! - depends: none
//! - verify: cargo test -p store
//! ```
//!
//! Unknown fields (refs, contract, tier, source…) are ignored here.

use xherd_core::domain::{Task, TaskStatus};

fn heading(line: &str) -> Option<(String, String)> {
    let rest = line.trim_start_matches('#');
    let level = line.len() - rest.len();
    if !(2..=4).contains(&level) || !rest.starts_with(' ') {
        return None;
    }
    let rest = rest.trim_start();
    let digits = rest.strip_prefix('T')?;
    let n = digits.chars().take_while(char::is_ascii_digit).count();
    if n == 0 {
        return None;
    }
    let id = format!("T{}", &digits[..n]);
    let title = digits[n..]
        .trim_start_matches([':', '.', ' ', '-', '—'])
        .trim()
        .to_string();
    Some((id, title))
}

/// The `T01` of a task heading.
pub(crate) fn heading_id(line: &str) -> Option<String> {
    heading(line).map(|(id, _)| id)
}

fn list(v: &str) -> Vec<String> {
    let v = v.trim();
    if matches!(v.to_ascii_lowercase().as_str(), "" | "none" | "-" | "—") {
        return Vec::new();
    }
    v.split(',')
        .map(|s| s.trim().trim_matches('`').trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn status(v: &str) -> Option<TaskStatus> {
    match v.trim().trim_matches('`').to_ascii_lowercase().as_str() {
        "todo" => Some(TaskStatus::Todo),
        "doing" => Some(TaskStatus::Doing),
        "done" => Some(TaskStatus::Done),
        "blocked" => Some(TaskStatus::Blocked),
        _ => None,
    }
}

/// `text` without the progress lines of task sections (`- status:` and
/// `- evidence:` under a task heading), byte for byte otherwise. Recording
/// progress is not a content change of the tasks document (R-107).
pub fn without_progress(text: &str) -> String {
    let mut in_task = false;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        if bare.starts_with('#') {
            in_task = heading(bare).is_some();
        } else if in_task {
            let item = bare.trim_start();
            let key = item
                .strip_prefix("- ")
                .or_else(|| item.strip_prefix("* "))
                .and_then(|i| i.split_once(':'))
                .map(|(k, _)| k.trim().to_ascii_lowercase());
            if matches!(key.as_deref(), Some("status" | "evidence")) {
                continue;
            }
        }
        out.push_str(line);
    }
    out
}

pub fn parse(text: &str) -> Result<Vec<Task>, String> {
    let mut tasks: Vec<Task> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let at = i + 1;
        if let Some((id, title)) = heading(line) {
            if tasks.iter().any(|t| t.id == id) {
                return Err(format!("tasks.md:{at}: duplicate task {id}"));
            }
            tasks.push(Task {
                id,
                title,
                status: TaskStatus::Todo,
                depends: Vec::new(),
                files: Vec::new(),
                evidence: None,
                verify: None,
                blocked_reason: None,
            });
            continue;
        }
        if line.starts_with('#') {
            // Any other heading ends the current task's field list.
            continue;
        }
        let (Some(t), Some(item)) = (
            tasks.last_mut(),
            line.trim_start()
                .strip_prefix("- ")
                .or_else(|| line.trim_start().strip_prefix("* ")),
        ) else {
            continue;
        };
        let Some((key, value)) = item.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "status" => {
                t.status = status(value).ok_or_else(|| {
                    format!("tasks.md:{at}: status {value:?} is not todo, doing, done or blocked")
                })?;
            }
            "files" => t.files = list(value),
            "depends" => t.depends = list(value),
            "evidence" if !value.is_empty() => t.evidence = Some(value.to_string()),
            "verify" if !value.is_empty() => t.verify = Some(value.trim_matches('`').to_string()),
            _ => {}
        }
    }
    if tasks.is_empty() {
        return Err("tasks.md: no tasks found (headings like `## T01: title`)".into());
    }
    for t in &tasks {
        if let Some(d) = t
            .depends
            .iter()
            .find(|d| !tasks.iter().any(|o| &o.id == *d))
        {
            return Err(format!("tasks.md: {} depends on unknown task {d}", t.id));
        }
    }
    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
---
ticket: SC-1
---
# Tasks

## T01: Add the store
- status: done
- files: `src/store.rs`, `src/lib.rs`
- depends: none
- evidence: unit tests pass
- refs: R1

## T02 — Wire the API
- status: todo
- files: src/api.rs
- depends: T01
- verify: `cargo test -p api`

## Assumptions
- none
";

    #[test]
    fn parses_tasks() {
        let t = parse(SAMPLE).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].id, "T01");
        assert_eq!(t[0].title, "Add the store");
        assert_eq!(t[0].status, TaskStatus::Done);
        assert_eq!(t[0].files, ["src/store.rs", "src/lib.rs"]);
        assert!(t[0].depends.is_empty());
        assert_eq!(t[0].evidence.as_deref(), Some("unit tests pass"));
        assert_eq!(t[1].title, "Wire the API");
        assert_eq!(t[1].depends, ["T01"]);
        assert_eq!(t[1].verify.as_deref(), Some("cargo test -p api"));
    }

    #[test]
    fn progress_lines_dropped() {
        let base = without_progress(SAMPLE);
        assert!(!base.contains("status:") && !base.contains("evidence:"));
        assert!(
            base.contains("- files: src/api.rs\n") && base.contains("## Assumptions\n- none\n")
        );
        let progressed = SAMPLE
            .replace(
                "- status: todo",
                "- status: done\n- evidence: api tests pass",
            )
            .replace(
                "- status: done\n- files: `src/store",
                "* Status: `doing`\n- files: `src/store",
            );
        assert_eq!(without_progress(&progressed), base);
        // Outside task sections, and other fields, are content.
        assert_eq!(
            without_progress("# Notes\n- status: x\n"),
            "# Notes\n- status: x\n"
        );
        assert_ne!(
            without_progress(&SAMPLE.replace("src/api.rs", "src/x.rs")),
            base
        );
    }

    #[test]
    fn errors() {
        assert!(parse("# nothing\n").is_err());
        assert!(
            parse("## T01: a\n## T01: b\n")
                .unwrap_err()
                .contains("duplicate")
        );
        assert!(
            parse("## T01: a\n- status: maybe\n")
                .unwrap_err()
                .contains(":2:")
        );
        assert!(
            parse("## T01: a\n- depends: T09\n")
                .unwrap_err()
                .contains("T09")
        );
    }
}
