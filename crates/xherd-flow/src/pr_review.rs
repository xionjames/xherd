//! xherd `pr-review.md` (FORMAT.md): the agent's decision about each new
//! comment on the pull request, one `###` heading per comment with its id and
//! a field list, like `review.md`:
//!
//! ```markdown
//! ### review-comment:456: Rename the helper
//! - decision: fix
//! - reason: the name hides that it writes to disk
//! - reply: Good point, renamed to `write_state`.
//! ```
//!
//! The user can change any decision before approving (R-306).

use xherd_core::pr::{Decision, Verdict};

/// The stage whose gate this document opens.
pub const STAGE: &str = "pr-review";

/// The id of a `###` heading: its first word without a trailing colon, so
/// `### issue:12: rename` and `### issue:12 rename` both give `issue:12`.
fn heading_id(line: &str) -> Option<String> {
    let rest = line.strip_prefix("###")?;
    if !rest.starts_with(' ') {
        return None;
    }
    let id = rest.split_whitespace().next()?.trim_end_matches(':');
    (!id.is_empty()).then(|| id.to_string())
}

fn decision(v: &str) -> Option<Decision> {
    match v.trim().trim_matches('`').to_ascii_lowercase().as_str() {
        "fix" => Some(Decision::Fix),
        "reply" => Some(Decision::Reply),
        "skip" => Some(Decision::Skip),
        _ => None,
    }
}

/// The verdicts of a `pr-review.md`. An error names the line.
pub fn parse(text: &str) -> Result<Vec<Verdict>, String> {
    // Verdict, the line of its heading, and whether it has a decision.
    let mut out: Vec<(Verdict, usize, bool)> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let at = i + 1;
        if let Some(id) = heading_id(line) {
            if out.iter().any(|(v, ..)| v.id == id) {
                return Err(format!("pr-review.md:{at}: duplicate comment {id}"));
            }
            out.push((
                Verdict {
                    id,
                    decision: Decision::Skip,
                    note: None,
                    reply: None,
                },
                at,
                false,
            ));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let (Some((v, _, has)), Some(item)) = (
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
            "decision" => {
                v.decision = decision(value).ok_or_else(|| {
                    format!(
                        "pr-review.md:{at}: decision {value:?} of {} is not fix, reply or skip",
                        v.id
                    )
                })?;
                *has = true;
            }
            "reason" if !value.is_empty() => v.note = Some(value.to_string()),
            "reply" if !value.is_empty() => v.reply = Some(value.to_string()),
            _ => {}
        }
    }
    if let Some((v, at, _)) = out.iter().find(|(_, _, has)| !has) {
        return Err(format!(
            "pr-review.md:{at}: {} has no `- decision: fix|reply|skip` line",
            v.id
        ));
    }
    Ok(out.into_iter().map(|(v, ..)| v).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decisions_by_comment_id() {
        let v = parse(
            "---\nstage: pr-review\n---\n## Summary\nx\n\n### review-comment:456: Rename the helper\n- decision: fix\n- reason: hides the write\n- reply: renamed\n\n### issue:12 question\n- decision: `Reply`\n- reply: it is intentional\n\n### issue:13\n- decision: skip\n",
        )
        .unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(
            (v[0].id.as_str(), v[0].decision),
            ("review-comment:456", Decision::Fix)
        );
        assert_eq!(v[0].note.as_deref(), Some("hides the write"));
        assert_eq!(
            (v[1].id.as_str(), v[1].decision),
            ("issue:12", Decision::Reply)
        );
        assert_eq!(v[1].reply.as_deref(), Some("it is intentional"));
        assert_eq!(v[2].decision, Decision::Skip);
    }

    #[test]
    fn bad_documents_name_the_line() {
        let e = parse("### issue:1\n- reason: x\n").unwrap_err();
        assert!(
            e.contains("pr-review.md:1") && e.contains("no `- decision"),
            "{e}"
        );
        let e = parse("### issue:1\n- decision: maybe\n").unwrap_err();
        assert!(
            e.contains("pr-review.md:2") && e.contains("not fix, reply or skip"),
            "{e}"
        );
        let e = parse("### a\n- decision: skip\n### a\n- decision: skip\n").unwrap_err();
        assert!(e.contains("duplicate comment a"), "{e}");
        assert_eq!(parse("## Summary\nNone.\n").unwrap(), []);
    }
}
