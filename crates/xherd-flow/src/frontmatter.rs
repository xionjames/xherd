//! YAML frontmatter at the top of a Markdown document (spec §7.3).

use serde_json::{Map, Value};

/// `Ok(None)` when the document has no frontmatter; `Err` when it has one that
/// is not a YAML mapping.
pub fn parse(text: &str) -> Result<Option<Map<String, Value>>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(body) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return Ok(None);
    };
    let mut yaml = String::new();
    for line in body.split_inclusive('\n') {
        if line.trim_end() == "---" {
            if yaml.trim().is_empty() {
                return Ok(Some(Map::new()));
            }
            return yaml_serde::from_str::<Map<String, Value>>(&yaml)
                .map(Some)
                .map_err(|e| format!("frontmatter: {e}"));
        }
        yaml.push_str(line);
    }
    Err("frontmatter: missing closing '---'".into())
}

/// A frontmatter value as text, for comparisons (`design: skip`, `needs-human: yes`).
pub fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_mapping() {
        let m = parse("---\nticket: SC-1\ndesign: skip\n---\n# Spec\n")
            .unwrap()
            .unwrap();
        assert_eq!(as_text(&m["ticket"]), "SC-1");
        assert_eq!(as_text(&m["design"]), "skip");
    }

    #[test]
    fn crlf_and_empty() {
        let m = parse("---\r\nflow: feature\r\n---\r\nbody")
            .unwrap()
            .unwrap();
        assert_eq!(as_text(&m["flow"]), "feature");
        assert!(parse("---\n---\n").unwrap().unwrap().is_empty());
    }

    #[test]
    fn none_and_errors() {
        assert_eq!(parse("# no frontmatter\n").unwrap(), None);
        assert!(parse("---\nticket: SC-1\n").is_err());
        assert!(parse("---\n- a list\n---\n").is_err());
    }

    #[test]
    fn non_strings_as_text() {
        let m = parse("---\nn: 3\nb: true\n---\n").unwrap().unwrap();
        assert_eq!(as_text(&m["n"]), "3");
        assert_eq!(as_text(&m["b"]), "true");
    }
}
