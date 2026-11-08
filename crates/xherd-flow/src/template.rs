//! `{name}` placeholders in pack commands and install paths. `${…}` is left
//! untouched (shell-style variables expanded elsewhere).

/// Replace every `{name}` with its value. Unknown names are an error, so a
/// typo in a pack is caught when the pack is loaded.
pub fn render(tpl: &str, vars: &[(&str, &str)]) -> Result<String, String> {
    let mut out = String::with_capacity(tpl.len());
    let mut rest = tpl;
    while let Some(i) = rest.find('{') {
        let dollar = rest[..i].ends_with('$');
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let end = after
            .find('}')
            .ok_or_else(|| format!("unclosed '{{' in {tpl:?}"))?;
        let name = &after[..end];
        if dollar {
            out.push('{');
            out.push_str(name);
            out.push('}');
        } else {
            let value = vars
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| *v)
                .ok_or_else(|| format!("unknown placeholder {{{name}}} in {tpl:?}"))?;
            out.push_str(value);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_known_names() {
        let s = render(
            "{prefix}feature {key} {prompt}",
            &[
                ("prefix", "/xherd:"),
                ("key", "SC-1"),
                ("prompt", "add login"),
            ],
        )
        .unwrap();
        assert_eq!(s, "/xherd:feature SC-1 add login");
    }

    #[test]
    fn keeps_shell_variables() {
        let s = render(
            "${CLAUDE_CONFIG_DIR:-~/.claude}/x/{name}.md",
            &[("name", "a")],
        )
        .unwrap();
        assert_eq!(s, "${CLAUDE_CONFIG_DIR:-~/.claude}/x/a.md");
    }

    #[test]
    fn rejects_unknown_and_unclosed() {
        assert!(render("{prefx}next", &[("prefix", "/x:")]).is_err());
        assert!(render("{prefix", &[("prefix", "/x:")]).is_err());
    }
}
