//! Open questions of a gate document (R-105a): the top-level list items under
//! the `## Open questions` heading. The document is the only source; the UI
//! answers each item with an inline comment on it.

/// The questions in `md`, in document order. A lone "None." (any case, period
/// optional) or an empty or missing section means none. An item's indented
/// continuation lines are joined to it with a space; nested lists are left out.
pub fn parse(md: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut inside = false;
    let mut fence = false;
    // The item being read accepts continuation lines.
    let mut open = false;
    for line in md.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            open = false;
            continue;
        }
        if fence {
            continue;
        }
        if let Some(h) = heading(line) {
            inside = h.eq_ignore_ascii_case("open questions");
            open = false;
            continue;
        }
        if !inside {
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if let (false, Some(text)) = (indented, item(line)) {
            out.push(text.to_string());
            open = true;
        } else if t.is_empty() {
            open = false;
        } else if indented && open && item(t).is_none() {
            if let Some(last) = out.last_mut() {
                last.push(' ');
                last.push_str(t.trim_end());
            }
        } else if !indented {
            open = false;
        }
    }
    out.retain(|q| !q.is_empty());
    if out.len() == 1 && is_none(&out[0]) {
        out.clear();
    }
    out
}

/// Two questions are the same when they differ only in spacing.
pub fn same(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

/// Text of an ATX heading of any level.
fn heading(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches('#');
    let level = line.len() - rest.len();
    ((1..=6).contains(&level) && (rest.is_empty() || rest.starts_with(' ')))
        .then(|| rest.trim().trim_end_matches('#').trim())
}

/// Text of a list item (`-`, `*`, `+` or `1.` / `1)`).
fn item(line: &str) -> Option<&str> {
    let rest = if let Some(r) = line.strip_prefix(['-', '*', '+']) {
        r
    } else {
        let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            return None;
        }
        line[digits..].strip_prefix(['.', ')'])?
    };
    (rest.is_empty() || rest.starts_with([' ', '\t'])).then(|| rest.trim())
}

fn is_none(q: &str) -> bool {
    q.trim_end_matches('.').eq_ignore_ascii_case("none")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_under_the_heading() {
        let md = "---\nstage: spec\n---\n# Spec\n\n## Requirements\n- R1: x\n\n\
                  ## Open questions\n- Keep expired sessions\n  for audit?\n  - nested note\n\
                  * Which db?\n1. Who owns it?\n\nA paragraph.\n\n## Assumptions\n- y\n";
        assert_eq!(
            parse(md),
            [
                "Keep expired sessions for audit?",
                "Which db?",
                "Who owns it?"
            ]
        );
    }

    #[test]
    fn none_empty_or_missing() {
        assert!(parse("## Open questions\n- None.\n").is_empty());
        assert!(parse("## Open questions\n- none\n").is_empty());
        assert!(parse("## Open questions\nNone.\n").is_empty());
        assert!(parse("## Open questions\n\n## Assumptions\n- a\n").is_empty());
        assert!(parse("# Design\n## Approach\n- a\n").is_empty());
        // "None" among real questions is a question like any other.
        assert_eq!(parse("## Open questions\n- a?\n- None.\n").len(), 2);
    }

    #[test]
    fn heading_level_case_and_fences() {
        let md = "### open questions ##\n```\n- not this\n## Open questions\n```\n- this?\n";
        assert_eq!(parse(md), ["this?"]);
        assert!(parse("##Open questions\n- x\n").is_empty());
        assert!(parse("## Open questions\n-x\n").is_empty());
    }

    #[test]
    fn same_ignores_spacing() {
        assert!(same("Which  db?\n", " Which db?"));
        assert!(!same("Which db?", "Which DB?"));
    }
}
