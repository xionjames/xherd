// The version shown to people, picked at build time. Included by `build.rs`
// and compiled for tests in `src/lib.rs`, so plain std only.

/// `v0.2.0-3-gabc1234[-dirty]` or `abc1234[-dirty]` → display version.
pub fn display(raw: &str, cargo: &str) -> String {
    let (raw, dirty) = match raw.strip_suffix("-dirty") {
        Some(r) => (r, true),
        None => (raw, false),
    };
    let mut parts = raw.rsplitn(3, '-');
    let (hash, n, tag) = (parts.next(), parts.next(), parts.next());
    let mut v = match (tag, n.and_then(|n| n.parse::<u32>().ok()), hash) {
        (Some(tag), Some(0), _) => tag.trim_start_matches('v').to_string(),
        (Some(tag), Some(n), Some(h)) => {
            format!(
                "{}+{n}.{}",
                tag.trim_start_matches('v'),
                h.trim_start_matches('g')
            )
        }
        _ => format!("{cargo}+{raw}"),
    };
    if dirty {
        v.push_str(if v.contains('+') { ".dirty" } else { "+dirty" });
    }
    v
}

/// R8, R10: a non-empty `XHERD_VERSION` wins, else what `git describe` gave,
/// else `dev` (never the Cargo version: a build without git is not a release).
pub fn pick(env: Option<&str>, described: Option<&str>, cargo: &str) -> String {
    match env.map(str::trim) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => described.map_or_else(|| "dev".to_string(), |d| display(d, cargo)),
    }
}

#[cfg(test)]
mod tests {
    use super::pick;

    #[test]
    fn env_wins() {
        assert_eq!(
            pick(Some(" dev-abc1234 "), Some("v0.2.0-0-gabc"), "0.1.0"),
            "dev-abc1234"
        );
        assert_eq!(pick(Some("  "), Some("v0.2.0-0-gabc"), "0.1.0"), "0.2.0");
    }

    #[test]
    fn git_cases_are_unchanged() {
        assert_eq!(pick(None, Some("v0.2.0-0-gabc1234"), "0.1.0"), "0.2.0");
        assert_eq!(
            pick(None, Some("v0.2.0-3-gabc1234"), "0.1.0"),
            "0.2.0+3.abc1234"
        );
        assert_eq!(pick(None, Some("abc1234"), "0.1.0"), "0.1.0+abc1234");
        assert_eq!(
            pick(None, Some("v0.2.0-3-gabc1234-dirty"), "0.1.0"),
            "0.2.0+3.abc1234.dirty"
        );
        assert_eq!(
            pick(None, Some("v0.2.0-0-gabc1234-dirty"), "0.1.0"),
            "0.2.0+dirty"
        );
        assert_eq!(
            pick(None, Some("abc1234-dirty"), "0.1.0"),
            "0.1.0+abc1234.dirty"
        );
    }

    #[test]
    fn neither_git_nor_env_is_dev() {
        assert_eq!(pick(None, None, "0.1.0"), "dev");
    }
}
