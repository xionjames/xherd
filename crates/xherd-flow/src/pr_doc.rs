//! Read and rewrite the PR title and body of `pr.md` (FORMAT.md, pr.md).
//!
//! The title is the `title:` frontmatter key, the body is everything after the
//! frontmatter. A rewrite touches only the line or the body it is asked to
//! change: every other byte of the frontmatter stays as it was.

use crate::frontmatter;

/// The title and body of a `pr.md`, as written (nothing normalised).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrFields {
    /// `None` when the key is missing, `null` or empty (counts as "at baseline").
    pub title: Option<String>,
    /// Everything after the closing `---` line and the blank lines that follow it.
    pub body: String,
}

/// Byte offsets of the frontmatter block in a text.
struct Block {
    /// Start of the first line after the opening `---`.
    yaml_start: usize,
    /// Start of the closing `---` line.
    close_start: usize,
    /// End of the block: after the closing line and the blank lines that follow.
    end: usize,
}

fn locate(text: &str) -> Option<Block> {
    let bom = if text.starts_with('\u{feff}') { 3 } else { 0 };
    let rest = &text[bom..];
    let open = if rest.starts_with("---\n") {
        4
    } else if rest.starts_with("---\r\n") {
        5
    } else {
        return None;
    };
    let yaml_start = bom + open;
    let mut pos = yaml_start;
    let mut lines = text[yaml_start..].split_inclusive('\n');
    for line in lines.by_ref() {
        if line.trim_end() == "---" {
            let close_start = pos;
            let mut end = pos + line.len();
            for blank in lines {
                if !blank.trim().is_empty() {
                    break;
                }
                end += blank.len();
            }
            return Some(Block {
                yaml_start,
                close_start,
                end,
            });
        }
        pos += line.len();
    }
    None
}

/// The title and body of `text`; `None` when it has no valid frontmatter.
pub fn fields(text: &str) -> Option<PrFields> {
    let map = frontmatter::parse(text).ok()??;
    let block = locate(text)?;
    let title = map
        .get("title")
        .map(frontmatter::as_text)
        .filter(|t| !t.trim().is_empty());
    Some(PrFields {
        title,
        body: text[block.end..].to_string(),
    })
}

/// `text` with its body replaced by `body` (and a final newline); the
/// frontmatter block is kept byte for byte. `None` when there is no valid
/// frontmatter.
pub fn with_body(text: &str, body: &str) -> Option<String> {
    fields(text)?;
    let block = locate(text)?;
    Some(format!("{}{body}\n", &text[..block.end]))
}

/// `text` with the frontmatter `title:` set to `title`: the key's line is
/// replaced, or one line is inserted before the closing `---`; no other byte
/// changes. `None` when there is no valid frontmatter or the result would not
/// read back as `title` (a broken file is never produced).
pub fn with_title(text: &str, title: &str) -> Option<String> {
    fields(text)?;
    let block = locate(text)?;
    let eol = if text[..block.yaml_start].ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let line = format!("title: {}{eol}", serde_json::to_string(title).ok()?);

    // The existing key: a top-level `title:` line and its indented lines.
    let mut start = None;
    let mut end = block.close_start;
    let mut pos = block.yaml_start;
    for l in text[block.yaml_start..block.close_start].split_inclusive('\n') {
        match start {
            None if l.starts_with("title:") => start = Some(pos),
            Some(_) if !l.starts_with([' ', '\t']) => {
                end = pos;
                break;
            }
            _ => {}
        }
        pos += l.len();
    }
    let (from, to) = match start {
        Some(s) => (s, end),
        None => (block.close_start, block.close_start),
    };
    let out = format!("{}{line}{}", &text[..from], &text[to..]);
    (fields(&out)?.title.as_deref() == Some(title)).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "---\nticket: SC-1\nflow: feature\nstage: ship\n# a comment\ntemplate: xherd\n---\n\n## Summary\nHello.\n";

    #[test]
    fn reads_title_and_body() {
        let t = "---\ntitle: Add it\nflow: feature\n---\n\n\n## Summary\nx\n";
        let f = fields(t).unwrap();
        assert_eq!(f.title.as_deref(), Some("Add it"));
        assert_eq!(f.body, "## Summary\nx\n");
        assert_eq!(fields(DOC).unwrap().title, None);
        assert_eq!(fields("# no frontmatter\n"), None);
        assert_eq!(fields("---\nticket: x\n"), None);
    }

    #[test]
    fn null_and_empty_title_are_none() {
        for t in [
            "title:",
            "title: null",
            "title: ~",
            "title: \"\"",
            "title: '  '",
        ] {
            let doc = format!("---\n{t}\n---\nbody\n");
            assert_eq!(fields(&doc).unwrap().title, None, "{t}");
        }
    }

    #[test]
    fn body_swap_keeps_the_frontmatter_bytes() {
        let out = with_body(DOC, "## Summary\nNew.").unwrap();
        let head = DOC.find("## Summary").unwrap();
        assert_eq!(&out[..head], &DOC[..head]);
        assert_eq!(&out[head..], "## Summary\nNew.\n");
        assert_eq!(with_body("no frontmatter", "x"), None);
    }

    #[test]
    fn body_swap_keeps_the_blank_lines_after_the_frontmatter() {
        let t = "---\na: 1\n---\n\n\nold\n";
        assert_eq!(with_body(t, "new").unwrap(), "---\na: 1\n---\n\n\nnew\n");
        let t = "---\na: 1\n---\nold\n";
        assert_eq!(with_body(t, "new").unwrap(), "---\na: 1\n---\nnew\n");
    }

    #[test]
    fn title_insert_keeps_everything_else() {
        let out = with_title(DOC, "Add the store").unwrap();
        assert_eq!(
            out,
            DOC.replace("---\n\n##", "title: \"Add the store\"\n---\n\n##")
        );
        let f = fields(&out).unwrap();
        assert_eq!(f.title.as_deref(), Some("Add the store"));
        assert_eq!(f.body, "## Summary\nHello.\n");
    }

    #[test]
    fn title_replace_keeps_other_keys_and_the_body() {
        let t = "---\nticket: SC-1\ntitle: Old one\nflow: feature\n---\n\nbody\n";
        assert_eq!(
            with_title(t, "New one").unwrap(),
            "---\nticket: SC-1\ntitle: \"New one\"\nflow: feature\n---\n\nbody\n"
        );
    }

    #[test]
    fn title_replace_drops_the_old_continuation_lines() {
        let t = "---\ntitle: >\n  folded\n  title\nflow: feature\n---\nbody\n";
        assert_eq!(
            with_title(t, "Short").unwrap(),
            "---\ntitle: \"Short\"\nflow: feature\n---\nbody\n"
        );
    }

    #[test]
    fn titles_with_yaml_metacharacters_round_trip() {
        for title in [
            "Fix: the \"quoted\" thing",
            "Use #1 first",
            "It's a 'test'",
            "Ünïcödé — 日本語 🚀",
            "- list-looking",
            "back\\slash",
            "ends with colon:",
            "{braces} [brackets] & *star",
            "tab\there",
        ] {
            let out = with_title(DOC, title).unwrap_or_else(|| panic!("{title}"));
            assert_eq!(fields(&out).unwrap().title.as_deref(), Some(title));
            let again = with_title(&out, title).unwrap();
            assert_eq!(again, out, "replacing with the same title changes nothing");
        }
    }

    #[test]
    fn crlf_frontmatter() {
        let t = "---\r\nticket: SC-1\r\n---\r\n\r\nbody\r\n";
        assert_eq!(
            with_title(t, "T").unwrap(),
            "---\r\nticket: SC-1\r\ntitle: \"T\"\r\n---\r\n\r\nbody\r\n"
        );
        assert_eq!(
            with_body(t, "new").unwrap(),
            "---\r\nticket: SC-1\r\n---\r\n\r\nnew\n"
        );
        assert_eq!(fields(t).unwrap().body, "body\r\n");
    }

    #[test]
    fn a_title_that_would_not_read_back_is_refused() {
        assert_eq!(with_title(DOC, ""), None);
        assert_eq!(with_title("no frontmatter", "T"), None);
    }

    #[test]
    fn closing_line_at_the_end_of_the_file() {
        let t = "---\nticket: SC-1\n---";
        assert_eq!(fields(t).unwrap().body, "");
        assert_eq!(
            with_title(t, "T").unwrap(),
            "---\nticket: SC-1\ntitle: \"T\"\n---"
        );
    }
}
