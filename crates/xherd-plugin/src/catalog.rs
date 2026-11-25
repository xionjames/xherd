//! The plugin catalogue: where listed plugins come from, how a listing is
//! normalised, merged and searched (R1, R5). Pure: nothing is fetched here.

use serde::{Deserialize, Serialize};

use crate::manifest::Kind;
use crate::source::split_github_tree;

/// Longest accepted source URL, in characters.
const MAX_SOURCE_LEN: usize = 500;

/// A configured catalogue source (R1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// GitHub repositories with the topic `xherd-plugin`.
    Github,
    /// An `https://` index file.
    Index(String),
}

impl Source {
    /// `github` is GitHub; anything else must be an `https://` URL with a
    /// host, no whitespace or control characters, at most 500 characters.
    pub fn parse(s: &str) -> Result<Source, String> {
        let s = s.trim();
        if s == "github" {
            return Ok(Source::Github);
        }
        if s.chars().count() > MAX_SOURCE_LEN {
            return Err(format!("longer than {MAX_SOURCE_LEN} characters"));
        }
        if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("contains whitespace or a control character".into());
        }
        let Some(rest) = s
            .get(..8)
            .filter(|p| p.eq_ignore_ascii_case("https://"))
            .map(|_| &s[8..])
        else {
            return Err("must be \"github\" or an https:// URL".into());
        };
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        if host.is_empty() || host.starts_with('@') {
            return Err("the URL has no host".into());
        }
        Ok(Source::Index(s.to_string()))
    }
}

/// One listed plugin. Text fields are untrusted: build them with
/// [`clean_text`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub url: String,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stars: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed_at: Option<String>,
    /// The configured source that listed it.
    pub source: String,
    /// Names of installed plugins that come from this entry; computed per
    /// request, never stored.
    #[serde(default)]
    pub installed: Vec<String>,
}

/// Drops control characters and keeps at most `max` characters, so catalogue
/// text stays short and on one line.
pub fn clean_text(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A repository URL in one canonical form, plus the sub-folder when the URL
/// was a GitHub `tree/<ref>/<path>` link: lower-case host, no trailing `/`
/// or `.git`, `git@host:o/r` as `https://host/o/r` (R5).
pub fn normalise(url: &str) -> (String, Option<String>) {
    let url = url.trim();
    let url = match url.strip_prefix("git@").and_then(|r| r.split_once(':')) {
        Some((host, rest)) => format!("https://{host}/{rest}"),
        None => url.to_string(),
    };
    let url = lower_host(&url);
    let (repo, _, tree_path) = split_github_tree(&url);
    let repo = repo.trim_end_matches('/');
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    (repo.to_string(), normalise_path(tree_path.as_deref()))
}

/// Lower-cases the host of `scheme://host/…`; other text is unchanged.
fn lower_host(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let end = rest.find('/').unwrap_or(rest.len());
    format!(
        "{scheme}://{}{}",
        rest[..end].to_ascii_lowercase(),
        &rest[end..]
    )
}

/// A sub-folder without leading `./` or trailing `/`; empty is `None`.
pub fn normalise_path(path: Option<&str>) -> Option<String> {
    let mut p = path?.trim();
    while let Some(r) = p.strip_prefix("./") {
        p = r;
    }
    let p = p.trim_matches('/');
    (!p.is_empty() && p != ".").then(|| p.to_string())
}

/// What makes two listings the same plugin: repository and sub-folder.
fn key(url: &str, path: Option<&str>) -> (String, Option<String>) {
    let (repo, tree_path) = normalise(url);
    (repo, normalise_path(path).or(tree_path))
}

/// Names of the installed plugins that come from `entry`. `records` are
/// `(name, url, path)`. An entry with a path matches records of the same
/// repository and path; one without matches every record of the repository.
pub fn installed_names(entry: &Entry, records: &[(String, String, Option<String>)]) -> Vec<String> {
    let (repo, path) = key(&entry.url, entry.path.as_deref());
    records
        .iter()
        .filter(|(_, url, rpath)| {
            let (r, p) = key(url, rpath.as_deref());
            r == repo && (path.is_none() || p == path)
        })
        .map(|(name, _, _)| name.clone())
        .collect()
}

/// Joins the sources in order; for each `(repo, path)` the first entry wins.
pub fn merge(per_source: Vec<Vec<Entry>>) -> Vec<Entry> {
    let mut seen = std::collections::HashSet::new();
    per_source
        .into_iter()
        .flatten()
        .filter(|e| seen.insert(key(&e.url, e.path.as_deref())))
        .collect()
}

/// Every whitespace-separated word of `q` appears, case-insensitive, in the
/// name, owner, description or kind. An empty `q` matches everything.
pub fn matches(entry: &Entry, q: &str) -> bool {
    let kind = entry
        .kind
        .and_then(|k| serde_json::to_value(k).ok())
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let hay = format!(
        "{} {} {} {kind}",
        entry.name,
        entry.owner,
        entry.description.as_deref().unwrap_or("")
    )
    .to_lowercase();
    q.split_whitespace()
        .all(|w| hay.contains(&w.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, url: &str, path: Option<&str>) -> Entry {
        Entry {
            name: name.into(),
            owner: "acme".into(),
            description: None,
            url: url.into(),
            git_ref: None,
            path: path.map(str::to_string),
            kind: None,
            stars: None,
            pushed_at: None,
            source: "github".into(),
            installed: vec![],
        }
    }

    fn rec(name: &str, url: &str, path: Option<&str>) -> (String, String, Option<String>) {
        (name.into(), url.into(), path.map(str::to_string))
    }

    #[test]
    fn source_parse() {
        assert_eq!(Source::parse("github"), Ok(Source::Github));
        assert_eq!(Source::parse(" github "), Ok(Source::Github));
        assert_eq!(
            Source::parse("https://example.org/c.json"),
            Ok(Source::Index("https://example.org/c.json".into()))
        );
        for bad in [
            "",
            "http://example.org/c.json",
            "ftp://example.org/c",
            "https://",
            "https:///x",
            "https://exa mple.org/c.json",
            "https://example.org/\u{7}",
            "git@github.com:o/r",
            "Github",
        ] {
            assert!(Source::parse(bad).is_err(), "{bad:?}");
        }
        assert!(Source::parse(&format!("https://e.org/{}", "a".repeat(500))).is_err());
        assert!(Source::parse(&format!("https://e.org/{}", "a".repeat(400))).is_ok());
    }

    #[test]
    fn clean_text_drops_controls_and_truncates() {
        assert_eq!(clean_text("a\u{1b}[31mb\nc\td", 100), "a[31mbcd");
        assert_eq!(clean_text("abcdef", 3), "abc");
        assert_eq!(clean_text("  hi  ", 10), "hi");
        assert_eq!(clean_text("ééé", 2), "éé");
    }

    #[test]
    fn normalise_urls() {
        let plain = ("https://github.com/o/r".to_string(), None);
        for url in [
            "https://github.com/o/r",
            "https://github.com/o/r.git",
            "https://github.com/o/r/",
            "https://GitHub.COM/o/r",
            "git@github.com:o/r.git",
            "git@GitHub.com:o/r",
            "  https://github.com/o/r.git/ ",
        ] {
            assert_eq!(normalise(url), plain, "{url:?}");
        }
        assert_eq!(
            normalise("https://github.com/o/r/tree/main/plugins/x/"),
            ("https://github.com/o/r".into(), Some("plugins/x".into()))
        );
        assert_eq!(
            normalise("https://GITHUB.com/o/r/tree/v1"),
            ("https://github.com/o/r".into(), None)
        );
        // The path of the repository keeps its case; only the host is lowered.
        assert_eq!(
            normalise("https://host.example/O/R").0,
            "https://host.example/O/R"
        );
        // ssh:// is not normalised.
        assert_eq!(
            normalise("ssh://git@host.example/o/r.git").0,
            "ssh://git@host.example/o/r"
        );
    }

    #[test]
    fn normalise_paths() {
        assert_eq!(normalise_path(Some("./x/")), Some("x".into()));
        assert_eq!(normalise_path(Some("/a/b/")), Some("a/b".into()));
        assert_eq!(normalise_path(Some("./")), None);
        assert_eq!(normalise_path(Some("")), None);
        assert_eq!(normalise_path(Some(".")), None);
        assert_eq!(normalise_path(None), None);
    }

    #[test]
    fn installed_with_and_without_path() {
        let records = vec![
            rec("a", "https://github.com/o/r.git", Some("plugins/a")),
            rec("b", "git@github.com:o/r", Some("plugins/b")),
            rec("c", "https://github.com/o/other", None),
        ];
        // No path: every record of the repository.
        let e = entry("r", "https://github.com/o/r", None);
        assert_eq!(installed_names(&e, &records), ["a", "b"]);
        // A path: only the same folder.
        let e = entry("a", "https://github.com/o/r", Some("./plugins/a/"));
        assert_eq!(installed_names(&e, &records), ["a"]);
        // A tree link carries its path.
        let e = entry("b", "https://github.com/o/r/tree/main/plugins/b", None);
        assert_eq!(installed_names(&e, &records), ["b"]);
        let e = entry("z", "https://github.com/o/none", None);
        assert!(installed_names(&e, &records).is_empty());
    }

    #[test]
    fn merge_keeps_the_earlier_source() {
        let mut first = entry("first", "https://github.com/o/r.git", None);
        first.source = "github".into();
        let mut dup = entry("dup", "https://GitHub.com/o/r/", None);
        dup.source = "https://e.org/c.json".into();
        let other = entry("other", "https://github.com/o/r", Some("sub"));
        let merged = merge(vec![vec![first], vec![dup, other]]);
        let names: Vec<_> = merged.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["first", "other"]);
    }

    #[test]
    fn matches_every_word() {
        let mut e = entry("xherd-forgejo", "https://github.com/acme/f", None);
        e.description = Some("Forgejo host plugin".into());
        e.kind = Some(Kind::Host);
        assert!(matches(&e, ""));
        assert!(matches(&e, "  "));
        assert!(matches(&e, "FORGE"));
        assert!(matches(&e, "acme host"));
        assert!(matches(&e, "forgejo  plugin"));
        assert!(!matches(&e, "forgejo gitlab"));
        assert!(!matches(&e, "tracker"));
    }
}
