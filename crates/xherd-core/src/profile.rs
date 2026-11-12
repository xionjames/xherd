//! Repo profiles (spec §16.1, R-135): per-repository settings, matched by the
//! repository's remote URL first, then by path. Pure: matching and
//! validation; storage is in `xherd-store`, use in the daemon.
//!
//! Every field but the name and the matches is optional; an unset field falls
//! back to the global setting (`config.json`).

use serde::{Deserialize, Serialize};

use crate::autonomy::Autonomy;

pub const MAX_NAME: usize = 60;
pub const MAX_MATCHES: usize = 20;
pub const MAX_INSTRUCTIONS: usize = 4000;
pub const MAX_TEST_COMMAND: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Profile {
    /// Assigned on creation; ignored on import.
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// Where it applies: remote URL patterns (`github.com/acme/*`, any scheme
    /// or `git@` form, `*` matches anything) or absolute path prefixes
    /// (`/work/acme`, `~/acme`).
    #[serde(default, rename = "match")]
    pub matches: Vec<String>,
    /// Branch new agents start from, when the request names none (R-135).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    /// Kept for flows that need them; nothing reads them yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staging_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotfix_base: Option<String>,
    /// Default autonomy of new agents (R-109).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autonomy: Option<Autonomy>,
    /// Tool-permission mode of new agents (R-002a).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<crate::permission::PermissionMode>,
    /// Runtime plugin of new agents (`opencode`, `codex`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Replace the global `sensitivePaths` (an empty list switches it off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitive_paths: Option<Vec<String>>,
    /// Replace the global `maxDiffLines`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_diff_lines: Option<usize>,
    /// Replace the global `agentTokenBudget` (0 = off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_token_budget: Option<u64>,
    /// One shell command run in the worktree at each commit proposal; a
    /// failure holds the commit for a human (R-115).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_command: Option<String>,
    /// Same, run before the test command (a linter or formatter check).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lint_command: Option<String>,
    /// Added to the agent's `instructions.md` (spec §16.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// A remote URL as `host/owner/repo`: no scheme, user, `.git` or trailing
/// slash, lower case (hosts do not care, and GitHub paths do not either).
pub fn normalize_url(url: &str) -> String {
    let u = url.trim();
    let u = u.split_once("://").map_or(u, |(_, rest)| rest);
    // `git@host:owner/repo` and `ssh://git@host/owner/repo`
    let u = u.rsplit_once('@').map_or(u, |(_, rest)| rest);
    let u = u.replacen(':', "/", 1);
    let u = u.trim_end_matches('/');
    u.strip_suffix(".git").unwrap_or(u).to_lowercase()
}

fn is_path_pattern(p: &str) -> bool {
    p.starts_with('/') || p.starts_with('~')
}

/// `*` matches any run of characters, `/` included; everything else literally.
fn wildcard(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let [first, rest @ ..] = parts.as_slice() else {
        return false;
    };
    let Some(mut at) = text.strip_prefix(first).map(|t| text.len() - t.len()) else {
        return false;
    };
    let Some((last, middle)) = rest.split_last() else {
        return text == *first;
    };
    for m in middle {
        match text[at..].find(m) {
            Some(i) => at += i + m.len(),
            None => return false,
        }
    }
    text.len() >= at + last.len() && text[at..].ends_with(last)
}

impl Profile {
    /// The most specific match of `remote` (a URL), the length of the
    /// pattern, or `None`.
    fn url_score(&self, remote: &str) -> Option<usize> {
        let remote = normalize_url(remote);
        self.matches
            .iter()
            .filter(|m| !is_path_pattern(m))
            .map(|m| normalize_url(m))
            .filter(|m| wildcard(m, &remote))
            .map(|m| m.len())
            .max()
    }

    /// Same for a local path: a prefix at a folder boundary (`home` replaces a
    /// leading `~`).
    fn path_score(&self, path: &str, home: Option<&str>) -> Option<usize> {
        let path = path.trim_end_matches('/');
        self.matches
            .iter()
            .filter(|m| is_path_pattern(m))
            .filter_map(|m| match (m.strip_prefix('~'), home) {
                (Some(rest), Some(h)) => Some(format!("{}{rest}", h.trim_end_matches('/'))),
                (Some(_), None) => None,
                (None, _) => Some(m.clone()),
            })
            .map(|m| m.trim_end_matches('/').to_string())
            .filter(|m| path == m || path.starts_with(&format!("{m}/")))
            .map(|m| m.len())
            .max()
    }
}

/// The profile for a repository (R-002): by remote URL first, the longest
/// pattern winning, then by path; a tie goes to the earlier profile. `None`
/// when nothing matches.
pub fn resolve<'a>(
    profiles: &'a [Profile],
    remote: Option<&str>,
    path: &str,
    home: Option<&str>,
) -> Option<&'a Profile> {
    let best = |score: &dyn Fn(&Profile) -> Option<usize>| {
        profiles
            .iter()
            .filter_map(|p| score(p).map(|s| (s, p)))
            .fold(None::<(usize, &Profile)>, |acc, (s, p)| match acc {
                Some((bs, _)) if bs >= s => acc,
                _ => Some((s, p)),
            })
            .map(|(_, p)| p)
    };
    remote
        .and_then(|r| best(&|p| p.url_score(r)))
        .or_else(|| best(&|p| p.path_score(path, home)))
}

fn plain(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(char::is_control)
}

fn check_branch(label: &str, b: &Option<String>) -> Result<(), String> {
    match b {
        Some(b)
            if b.is_empty()
                || b.len() > 100
                || b.starts_with('-')
                || b.contains("..")
                || !b
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c)) =>
        {
            Err(format!("{label}: a plain branch name"))
        }
        _ => Ok(()),
    }
}

/// Trim a command; empty means unset; one line, bounded.
fn clean_command(field: &str, c: &mut Option<String>) -> Result<(), String> {
    *c = c
        .take()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());
    if let Some(c) = c
        && (c.len() > MAX_TEST_COMMAND || c.contains('\n') || c.contains('\0'))
    {
        return Err(format!(
            "{field}: one line, up to {MAX_TEST_COMMAND} characters"
        ));
    }
    Ok(())
}

/// Clean up and validate a profile the user sent (trims, drops empty and
/// repeated entries). `others` are the names of the other profiles.
pub fn check(p: &mut Profile, others: &[&str]) -> Result<(), String> {
    p.name = p.name.trim().to_string();
    p.runtime = p
        .runtime
        .take()
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty());
    if !plain(&p.name) || p.name.chars().count() > MAX_NAME {
        return Err(format!("name: 1 to {MAX_NAME} characters"));
    }
    if others.iter().any(|o| o.eq_ignore_ascii_case(&p.name)) {
        return Err(format!("a profile named {:?} exists", p.name));
    }
    let mut ms: Vec<String> = Vec::new();
    for m in &p.matches {
        let m = m.trim();
        if m.is_empty() {
            continue;
        }
        if !plain(m) || m.len() > 200 || m.contains(char::is_whitespace) {
            return Err("match: short URL patterns or absolute paths, no spaces".into());
        }
        if !is_path_pattern(m) && !m.contains('/') && !m.contains(':') {
            return Err(format!(
                "match {m:?}: use host/owner/repo or an absolute path"
            ));
        }
        if !ms.iter().any(|x| x == m) {
            ms.push(m.to_string());
        }
    }
    if ms.is_empty() || ms.len() > MAX_MATCHES {
        return Err(format!("match: 1 to {MAX_MATCHES} entries"));
    }
    p.matches = ms;
    check_branch("baseBranch", &p.base_branch)?;
    check_branch("stagingBranch", &p.staging_branch)?;
    check_branch("hotfixBase", &p.hotfix_base)?;
    if let Some(a) = p.autonomy
        && !a.selectable()
    {
        return Err("autonomy never is not available yet (v1.x, R-1209)".into());
    }
    if let Some(globs) = &p.sensitive_paths {
        let mut out: Vec<String> = Vec::new();
        for g in globs {
            let g = g.trim();
            if g.is_empty() || g.len() > 200 || !plain(g) || g.starts_with('/') {
                return Err("sensitivePaths: short relative globs, one per entry".into());
            }
            if !out.iter().any(|o| o == g) {
                out.push(g.to_string());
            }
        }
        p.sensitive_paths = Some(out);
    }
    if let Some(n) = p.max_diff_lines
        && !(1..=1_000_000).contains(&n)
    {
        return Err("maxDiffLines must be 1 to 1000000".into());
    }
    clean_command("testCommand", &mut p.test_command)?;
    clean_command("lintCommand", &mut p.lint_command)?;
    p.instructions = p
        .instructions
        .take()
        .map(|i| i.trim().to_string())
        .filter(|i| !i.is_empty());
    if let Some(i) = &p.instructions
        && (i.chars().count() > MAX_INSTRUCTIONS || i.contains('\0'))
    {
        return Err(format!("instructions: up to {MAX_INSTRUCTIONS} characters"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prof(name: &str, matches: &[&str]) -> Profile {
        Profile {
            id: name.into(),
            name: name.into(),
            matches: matches.iter().map(|s| s.to_string()).collect(),
            ..Profile::default()
        }
    }

    #[test]
    fn urls_normalise() {
        for u in [
            "https://github.com/Acme/App.git",
            "git@github.com:acme/app.git",
            "ssh://git@github.com/acme/app",
            "https://user:tok@github.com/acme/app/",
        ] {
            assert_eq!(normalize_url(u), "github.com/acme/app", "{u}");
        }
    }

    #[test]
    fn wildcards() {
        assert!(wildcard("github.com/acme/*", "github.com/acme/app"));
        assert!(wildcard("github.com/*/app", "github.com/acme/app"));
        assert!(wildcard("*", "anything"));
        assert!(wildcard("a*b*c", "aXXbYYc"));
        assert!(!wildcard("github.com/acme/*", "github.com/other/app"));
        assert!(!wildcard("github.com/acme/app", "github.com/acme/app2"));
        assert!(!wildcard("a*b", "a"));
    }

    #[test]
    fn url_beats_path_and_longest_wins() {
        let ps = [
            prof("work", &["github.com/acme/*"]),
            prof("app", &["git@github.com:acme/app.git"]),
            prof("dir", &["/work/acme"]),
        ];
        let name = |r, p| resolve(&ps, r, p, None).map(|p| p.name.as_str());
        assert_eq!(name(Some("https://github.com/acme/app"), "/x"), Some("app"));
        assert_eq!(
            name(Some("https://github.com/acme/other"), "/x"),
            Some("work")
        );
        // no remote match: the path decides
        assert_eq!(
            name(Some("https://gitlab.com/a/b"), "/work/acme/app"),
            Some("dir")
        );
        assert_eq!(name(None, "/work/acme"), Some("dir"));
        assert_eq!(name(None, "/work/acme2"), None);
    }

    #[test]
    fn tilde_needs_a_home() {
        let ps = [prof("h", &["~/acme"])];
        assert!(resolve(&ps, None, "/home/me/acme/x", Some("/home/me")).is_some());
        assert!(resolve(&ps, None, "/home/me/acme/x", None).is_none());
    }

    #[test]
    fn check_cleans_and_refuses() {
        let mut p = prof("A", &[" github.com/acme/* ", "", "github.com/acme/*"]);
        p.test_command = Some("  cargo test ".into());
        p.lint_command = Some("  ".into());
        p.sensitive_paths = Some(vec![" infra/** ".into()]);
        assert_eq!(check(&mut p, &[]), Ok(()));
        assert_eq!(p.matches, ["github.com/acme/*"]);
        assert_eq!(p.test_command.as_deref(), Some("cargo test"));
        assert_eq!(p.lint_command, None);
        assert_eq!(p.sensitive_paths, Some(vec!["infra/**".to_string()]));

        let bad = |f: fn(&mut Profile)| {
            let mut p = prof("A", &["github.com/acme/*"]);
            f(&mut p);
            check(&mut p, &["b"]).is_err()
        };
        assert!(bad(|p| p.name = " ".into()));
        assert!(bad(|p| p.name = "B".into())); // taken, case-insensitive
        assert!(bad(|p| p.matches = vec![]));
        assert!(bad(|p| p.matches = vec!["acme".into()]));
        assert!(bad(|p| p.base_branch = Some("-x".into())));
        assert!(bad(|p| p.base_branch = Some("a b".into())));
        assert!(bad(|p| p.max_diff_lines = Some(0)));
        assert!(bad(|p| p.autonomy = Some(Autonomy::Never)));
        assert!(bad(|p| p.sensitive_paths = Some(vec!["/etc".into()])));
        assert!(bad(|p| p.test_command = Some("a\nb".into())));
        assert!(bad(|p| p.lint_command = Some("a\nb".into())));
    }

    #[test]
    fn json_is_camel_case_with_match() {
        let p: Profile = serde_json::from_str(
            r#"{"name":"A","match":["github.com/a/b"],"baseBranch":"develop","maxDiffLines":5}"#,
        )
        .unwrap();
        assert_eq!(p.base_branch.as_deref(), Some("develop"));
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["match"][0], "github.com/a/b");
        assert!(serde_json::from_str::<Profile>(r#"{"name":"A","bogus":1}"#).is_err());
    }
}
