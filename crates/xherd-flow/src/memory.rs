//! xherd `memory.md` (FORMAT.md): only its structure is checked, so a fresh
//! session never starts from a missing or empty note.

use xherd_core::xap::{Refusal, refuse};

const SECTIONS: [&str; 4] = ["state", "decisions", "code map", "watch out"];
/// The format asks for about 60 lines.
const MAX_LINES: usize = 80;

/// `memory_invalid` unless `text` (`None` = file missing) has the four `##`
/// sections and at most [`MAX_LINES`] lines.
pub fn check(text: Option<&str>) -> Result<(), Refusal> {
    let invalid = |what: String| {
        refuse(
            "memory_invalid",
            format!("memory.md {what} (FORMAT.md, memory.md section); fix it before the gate"),
        )
    };
    let text = text.ok_or_else(|| invalid("is missing".into()))?;
    let headings: Vec<String> = text
        .lines()
        .filter_map(|l| l.strip_prefix("## "))
        .map(|h| h.trim().to_ascii_lowercase())
        .collect();
    let missing: Vec<&str> = SECTIONS
        .iter()
        .copied()
        .filter(|s| !headings.iter().any(|h| h == s))
        .collect();
    if !missing.is_empty() {
        let names: Vec<String> = missing.iter().map(|s| format!("`## {s}`")).collect();
        return Err(invalid(format!("lacks {}", names.join(", "))));
    }
    let lines = text.lines().count();
    if lines > MAX_LINES {
        return Err(invalid(format!(
            "has {lines} lines; keep it to about 60 (at most {MAX_LINES})"
        )));
    }
    Ok(())
}

/// Shared memory (spec §16.6) lives in `AGENTS.md`; Claude Code reads
/// `CLAUDE.md`, so that file must import it. The new `CLAUDE.md` text when it
/// is missing or lacks the `@AGENTS.md` line, else `None` (its content is
/// kept, the line appended).
pub fn claude_md(existing: Option<&str>) -> Option<String> {
    const IMPORT: &str = "@AGENTS.md";
    match existing {
        None => Some(format!("{IMPORT}\n")),
        Some(t) if t.lines().any(|l| l.trim() == IMPORT) => None,
        Some(t) if t.trim().is_empty() => Some(format!("{IMPORT}\n")),
        Some(t) => Some(format!("{}\n\n{IMPORT}\n", t.trim_end())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "# Memory\n## State\nx\n## decisions\n## Code map\n## Watch out \n";

    #[test]
    fn claude_md_imports_agents_md_once() {
        assert_eq!(claude_md(None).unwrap(), "@AGENTS.md\n");
        assert_eq!(claude_md(Some(" \n")).unwrap(), "@AGENTS.md\n");
        assert_eq!(
            claude_md(Some("# Notes\nkeep\n\n")).unwrap(),
            "# Notes\nkeep\n\n@AGENTS.md\n"
        );
        assert_eq!(claude_md(Some("x\n @AGENTS.md \n")), None);
    }

    #[test]
    fn structure_only() {
        assert!(check(Some(GOOD)).is_ok());
        assert!(
            check(Some(
                "## Watch out\n## Code map\r\n## Decisions\n## STATE\n"
            ))
            .is_ok()
        );
    }

    #[test]
    fn missing_file_section_or_too_long() {
        assert!(check(None).unwrap_err().message.contains("is missing"));
        let e = check(Some("## State\n## Decisions\n### Code map\n")).unwrap_err();
        assert_eq!(e.code, "memory_invalid");
        assert!(
            e.message.contains("`## code map`, `## watch out`"),
            "{}",
            e.message
        );
        let long = format!("{GOOD}{}", "x\n".repeat(80));
        assert!(check(Some(&long)).unwrap_err().message.contains("lines"));
        let ok = format!("{GOOD}{}", "x\n".repeat(70));
        assert!(check(Some(&ok)).is_ok());
    }
}
