//! Where an external plugin comes from: a remote git URL, an optional ref
//! and an optional sub-folder (R1). Pure: nothing is fetched here.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use xherd_core::new::clone_path;
use xherd_core::xap::{Refusal, refuse};

static REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._/-]*$").unwrap());
static SEGMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_][A-Za-z0-9._-]*$").unwrap());

/// A checked plugin source (R1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginSource {
    /// Remote URL git can fetch (`https`, `ssh` or scp-like).
    pub url: String,
    /// Branch, tag or commit; `None` is the default branch.
    pub git_ref: Option<String>,
    /// Sub-folder holding the manifest, `/`-separated, no leading slash.
    pub path: Option<String>,
}

fn bad(message: impl Into<String>) -> Refusal {
    refuse("bad_plugin_source", message)
}

impl PluginSource {
    /// Checks `url` with the rules of repository clones plus: no `http://`
    /// (code that will run must come over TLS or SSH) and no leading `-`.
    /// GitHub's `https://github.com/<o>/<r>/tree/<ref>/<path>` is split into
    /// URL, ref and path; an explicit `git_ref` or `path` wins over it.
    pub fn parse(
        url: &str,
        git_ref: Option<&str>,
        path: Option<&str>,
    ) -> Result<PluginSource, Refusal> {
        let url = url.trim();
        if url.starts_with('-') {
            return Err(bad("the URL must not start with '-'"));
        }
        if url.len() >= 7 && url[..7].eq_ignore_ascii_case("http://") {
            return Err(bad("http:// is refused: use https:// or ssh"));
        }
        let (url, tree_ref, tree_path) = split_github_tree(url);
        clone_path(Path::new("/"), &url).map_err(|e| bad(e.message))?;

        let git_ref = git_ref
            .map(str::to_string)
            .filter(|r| !r.is_empty())
            .or(tree_ref);
        if let Some(r) = &git_ref
            && (!REF.is_match(r) || r.contains("..") || r.contains("//") || r.ends_with('/'))
        {
            return Err(bad(format!("{r:?} is not a valid branch, tag or commit")));
        }
        let path = path
            .map(str::to_string)
            .filter(|p| !p.is_empty())
            .or(tree_path)
            .map(|p| p.trim_matches('/').to_string())
            .filter(|p| !p.is_empty());
        if let Some(p) = &path
            && !p.split('/').all(|s| SEGMENT.is_match(s) && s != "..")
        {
            return Err(bad(format!("{p:?} is not a plain sub-folder path")));
        }
        Ok(PluginSource { url, git_ref, path })
    }
}

/// `https://github.com/o/r/tree/<ref>/<path…>` → (`https://github.com/o/r.git`,
/// ref, path). Any other URL comes back unchanged. A ref with a slash is
/// ambiguous in this form: only its first segment is taken (pass `--ref`).
pub(crate) fn split_github_tree(url: &str) -> (String, Option<String>, Option<String>) {
    let Some(rest) = url.strip_prefix("https://github.com/") else {
        return (url.to_string(), None, None);
    };
    let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
    if parts.len() >= 4 && parts[2] == "tree" {
        let repo = parts[1].strip_suffix(".git").unwrap_or(parts[1]);
        let path = parts[4..].join("/");
        return (
            format!("https://github.com/{}/{repo}.git", parts[0]),
            Some(parts[3].to_string()),
            (!path.is_empty()).then_some(path),
        );
    }
    (url.to_string(), None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(url: &str) -> PluginSource {
        PluginSource::parse(url, None, None).unwrap()
    }

    fn refused(url: &str, r: Option<&str>, p: Option<&str>) -> &'static str {
        let e = PluginSource::parse(url, r, p).unwrap_err();
        e.code
    }

    #[test]
    fn plain_urls() {
        let s = ok("https://github.com/o/r");
        assert_eq!(s.url, "https://github.com/o/r");
        assert_eq!((s.git_ref, s.path), (None, None));
        assert_eq!(
            ok("https://github.com/o/r.git").url,
            "https://github.com/o/r.git"
        );
        assert_eq!(ok("git@github.com:o/r.git").url, "git@github.com:o/r.git");
        assert_eq!(
            ok("ssh://git@host.example/o/r.git").url,
            "ssh://git@host.example/o/r.git"
        );
    }

    #[test]
    fn github_tree_form() {
        let s = ok("https://github.com/o/r/tree/main/plugins/x");
        assert_eq!(s.url, "https://github.com/o/r.git");
        assert_eq!(s.git_ref.as_deref(), Some("main"));
        assert_eq!(s.path.as_deref(), Some("plugins/x"));
        let s = ok("https://github.com/o/r/tree/v1.2/");
        assert_eq!(s.git_ref.as_deref(), Some("v1.2"));
        assert_eq!(s.path, None);
    }

    #[test]
    fn explicit_ref_and_path_win() {
        let s = PluginSource::parse(
            "https://github.com/o/r/tree/main/a",
            Some("dev"),
            Some("b/c"),
        )
        .unwrap();
        assert_eq!(s.git_ref.as_deref(), Some("dev"));
        assert_eq!(s.path.as_deref(), Some("b/c"));
        let s = PluginSource::parse("https://github.com/o/r", Some(""), Some("/")).unwrap();
        assert_eq!((s.git_ref, s.path), (None, None));
    }

    #[test]
    fn refused_urls() {
        for url in [
            "http://github.com/o/r",
            "HTTP://github.com/o/r",
            "file:///tmp/repo",
            "/abs/path/repo",
            "ext::sh -c id",
            "-x@host:o/r",
            "--upload-pack=x",
            "https://github.com/o/\u{7}r",
            "https://github.com/o/r\nx",
            "https://github.com/o",
            "https://github.com/o/../r",
            "",
        ] {
            assert_eq!(refused(url, None, None), "bad_plugin_source", "{url:?}");
        }
        // Trailing whitespace is trimmed, an embedded control character is not.
        assert!(PluginSource::parse("https://github.com/o/r\n", None, None).is_ok());
    }

    #[test]
    fn refused_refs_and_paths() {
        for r in ["-x", "a..b", "a//b", "main/", "a b", "x;y", "$(id)"] {
            assert_eq!(
                refused("https://github.com/o/r", Some(r), None),
                "bad_plugin_source",
                "{r:?}"
            );
        }
        for p in ["../x", "a/../b", "a/./b", ".hidden", "a b", "a\\b", "x;y"] {
            assert_eq!(
                refused("https://github.com/o/r", None, Some(p)),
                "bad_plugin_source",
                "{p:?}"
            );
        }
        assert_eq!(
            refused("https://github.com/o/r/tree/-x/a", None, None),
            "bad_plugin_source"
        );
        assert_eq!(
            refused("https://github.com/o/r/tree/main/../x", None, None),
            "bad_plugin_source"
        );
    }

    #[test]
    fn commit_sha_is_a_valid_ref() {
        let sha = "3f2a9c1d4e5b6a7988776655443322110fedcba9";
        let s = PluginSource::parse("https://github.com/o/r", Some(sha), None).unwrap();
        assert_eq!(s.git_ref.as_deref(), Some(sha));
    }
}
