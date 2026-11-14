//! Model names chosen for a runtime (issue #112): per agent, per repo profile
//! (`models`) and per runtime in the config (`runtimeModels`). Pure.

use std::collections::BTreeMap;

pub const MAX_MODEL: usize = 100;

/// Trim a model name and check it: it goes on a command line, so letters,
/// digits and `. _ : / @ - [ ]` only, not starting with `-`. `None` = empty
/// (no model: the runtime's default).
pub fn clean(name: &str) -> Result<Option<String>, String> {
    let m = name.trim();
    if m.is_empty() {
        return Ok(None);
    }
    let plain = m.len() <= MAX_MODEL
        && !m.starts_with('-')
        && m.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:/@-[]".contains(c));
    if !plain {
        return Err(format!(
            "model {m:?}: letters, digits and . _ : / @ - [ ] only, up to {MAX_MODEL}"
        ));
    }
    Ok(Some(m.to_string()))
}

/// A runtime → model map as the user sent it: names trimmed and checked,
/// empty models dropped.
pub fn clean_map(map: BTreeMap<String, String>) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for (rt, m) in map {
        let rt = rt.trim().to_string();
        if let Some(m) = clean(&m).map_err(|e| format!("{rt}: {e}"))? {
            if rt.is_empty() || rt.len() > 60 {
                return Err("a runtime plugin name is needed".into());
            }
            out.insert(rt, m);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_only() {
        assert_eq!(clean(" sonnet ").unwrap().as_deref(), Some("sonnet"));
        assert_eq!(clean("").unwrap(), None);
        for ok in [
            "opencode/big-pickle",
            "claude-opus-5-5[1m]",
            "m@2025",
            "gpt-5.1:high",
        ] {
            assert!(clean(ok).unwrap().is_some(), "{ok}");
        }
        for bad in ["--dangerous", "a b", "x;rm", "$(x)", &"m".repeat(101)] {
            assert!(clean(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn maps_drop_empty_models() {
        let m = BTreeMap::from([
            ("claude-code".to_string(), " opus ".to_string()),
            ("codex".to_string(), "".to_string()),
        ]);
        let out = clean_map(m).unwrap();
        assert_eq!(out, BTreeMap::from([("claude-code".into(), "opus".into())]));
        let bad = BTreeMap::from([("codex".to_string(), "-x".to_string())]);
        assert!(clean_map(bad).is_err());
    }
}
