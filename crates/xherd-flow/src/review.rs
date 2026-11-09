//! xherd `review.md` findings (FORMAT.md): each finding is a heading with an
//! id and a bullet list of fields, like `tasks.md`:
//!
//! ```markdown
//! ### F01: Missing error handling in the store
//! - severity: major
//! - location: `src/store.rs:42`
//! - fixed: T05
//! ```
//!
//! `fixed` lists the tasks that fix it; `left` says why it is left as it is.
//! Everything else in the document is free text.

use xherd_core::domain::{Finding, Severity, Task, TaskStatus};

fn heading(line: &str) -> Option<(String, String)> {
    let rest = line.trim_start_matches('#');
    let level = line.len() - rest.len();
    if !(2..=4).contains(&level) || !rest.starts_with(' ') {
        return None;
    }
    let digits = rest.trim_start().strip_prefix('F')?;
    let n = digits.chars().take_while(char::is_ascii_digit).count();
    if n == 0 {
        return None;
    }
    let title = digits[n..]
        .trim_start_matches([':', '.', ' ', '-', '—'])
        .trim()
        .to_string();
    Some((format!("F{}", &digits[..n]), title))
}

fn severity(v: &str) -> Option<Severity> {
    match v.trim().trim_matches('`').to_ascii_lowercase().as_str() {
        "minor" => Some(Severity::Minor),
        "major" => Some(Severity::Major),
        "blocker" => Some(Severity::Blocker),
        _ => None,
    }
}

fn ids(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().trim_matches('`').trim().to_string())
        .filter(|s| s.starts_with('T') && s[1..].chars().all(|c| c.is_ascii_digit()) && s.len() > 1)
        .collect()
}

/// The findings of a `review.md`. An error names the line, so the agent can
/// fix the document before opening the gate.
pub fn parse(text: &str) -> Result<Vec<Finding>, String> {
    // Finding and the line of its heading, until its severity is known.
    let mut out: Vec<(Finding, usize, bool)> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let at = i + 1;
        if let Some((id, title)) = heading(line) {
            if out.iter().any(|(f, ..)| f.id == id) {
                return Err(format!("review.md:{at}: duplicate finding {id}"));
            }
            out.push((
                Finding {
                    id,
                    title,
                    severity: Severity::Minor,
                    location: None,
                    fixed: Vec::new(),
                    left: None,
                },
                at,
                false,
            ));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let (Some((f, _, has_sev)), Some(item)) = (
            out.last_mut(),
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
            "severity" => {
                f.severity = severity(value).ok_or_else(|| {
                    format!(
                        "review.md:{at}: severity {value:?} of {} is not minor, major or blocker",
                        f.id
                    )
                })?;
                *has_sev = true;
            }
            "location" if !value.is_empty() => f.location = Some(value.trim_matches('`').into()),
            "fixed" => f.fixed = ids(value),
            "left" if !value.is_empty() => f.left = Some(value.to_string()),
            _ => {}
        }
    }
    if let Some((f, at, _)) = out.iter().find(|(_, _, has)| !has) {
        return Err(format!(
            "review.md:{at}: {} has no `- severity: minor|major|blocker` line",
            f.id
        ));
    }
    Ok(out.into_iter().map(|(f, ..)| f).collect())
}

/// Fixed: it names tasks and all of them are done.
pub fn resolved(f: &Finding, tasks: &[Task]) -> bool {
    !f.fixed.is_empty()
        && f.fixed.iter().all(|id| {
            tasks
                .iter()
                .any(|t| &t.id == id && t.status == TaskStatus::Done)
        })
}

/// Findings that hold the review gate (R-114): major or blocker, not fixed.
pub fn open_findings<'a>(findings: &'a [Finding], tasks: &[Task]) -> Vec<&'a Finding> {
    findings
        .iter()
        .filter(|f| f.severity.holds() && !resolved(f, tasks))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use xherd_core::domain::TaskSource;

    const DOC: &str = "\
---
stage: review
---
## Summary
Fine.

## Findings

### F01: Missing error handling
- severity: major
- location: `src/store.rs:42`
- fixed: T05, T06

### F02 — Typo in a comment
- severity: `Minor`
- left: not worth a task

### F03: Race on startup
- severity: blocker
";

    fn task(id: &str, status: TaskStatus) -> Task {
        Task {
            id: id.into(),
            title: id.into(),
            status,
            depends: vec![],
            files: vec![],
            evidence: None,
            verify: None,
            blocked_reason: None,
            source: TaskSource::Review,
        }
    }

    #[test]
    fn parses_findings_and_ignores_the_rest() {
        let f = parse(DOC).unwrap();
        assert_eq!(f.len(), 3);
        assert_eq!((f[0].id.as_str(), f[0].severity), ("F01", Severity::Major));
        assert_eq!(f[0].location.as_deref(), Some("src/store.rs:42"));
        assert_eq!(f[0].fixed, ["T05", "T06"]);
        assert_eq!(f[0].title, "Missing error handling");
        assert_eq!(
            (f[1].title.as_str(), f[1].severity),
            ("Typo in a comment", Severity::Minor)
        );
        assert_eq!(f[1].left.as_deref(), Some("not worth a task"));
        assert_eq!(parse("## Findings\nNone.\n").unwrap(), []);
    }

    #[test]
    fn bad_documents_name_the_line() {
        let e = parse("### F01: x\n- location: a\n").unwrap_err();
        assert!(
            e.contains("review.md:1") && e.contains("no `- severity"),
            "{e}"
        );
        let e = parse("### F01: x\n- severity: critical\n").unwrap_err();
        assert!(
            e.contains("review.md:2") && e.contains("not minor, major or blocker"),
            "{e}"
        );
        let e =
            parse("### F01: x\n- severity: minor\n### F01: y\n- severity: minor\n").unwrap_err();
        assert!(e.contains("duplicate finding F01"), "{e}");
    }

    #[test]
    fn open_findings_are_major_or_blocker_not_fixed() {
        let f = parse(DOC).unwrap();
        let mut tasks = vec![
            task("T05", TaskStatus::Done),
            task("T06", TaskStatus::Doing),
        ];
        let open: Vec<_> = open_findings(&f, &tasks)
            .iter()
            .map(|f| f.id.clone())
            .collect();
        assert_eq!(
            open,
            ["F01", "F03"],
            "F01 waits for T06, F03 has no fix, F02 is minor"
        );
        tasks[1].status = TaskStatus::Done;
        let open: Vec<_> = open_findings(&f, &tasks)
            .iter()
            .map(|f| f.id.clone())
            .collect();
        assert_eq!(open, ["F03"]);
    }
}
