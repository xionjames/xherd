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

/// Longest kept entry name and description, in characters.
const NAME_MAX: usize = 100;
const DESCRIPTION_MAX: usize = 300;

fn description(v: Option<&str>) -> Option<String> {
    v.map(|d| clean_text(d, DESCRIPTION_MAX))
        .filter(|d| !d.is_empty())
}

/// The entries of a GitHub repository search answer (`items[]`). Only
/// repositories whose `clone_url` is on `https://github.com/` are kept (R2).
pub fn parse_github(body: &[u8], source: &str) -> Result<Vec<Entry>, String> {
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("not JSON: {e}"))?;
    let items = v
        .get("items")
        .and_then(|i| i.as_array())
        .ok_or("no \"items\" list in the answer")?;
    Ok(items
        .iter()
        .filter_map(|it| {
            let url = it.get("clone_url")?.as_str()?;
            if !url.starts_with("https://github.com/") {
                return None;
            }
            let name = clean_text(it.get("name")?.as_str()?, NAME_MAX);
            if name.is_empty() {
                return None;
            }
            let owner = it
                .pointer("/owner/login")
                .and_then(|o| o.as_str())
                .map(|o| clean_text(o, NAME_MAX))
                .filter(|o| !o.is_empty())
                .unwrap_or_else(|| "github.com".into());
            Some(Entry {
                name,
                owner,
                description: description(it.get("description").and_then(|d| d.as_str())),
                url: url.to_string(),
                git_ref: None,
                path: None,
                kind: None,
                stars: it.get("stargazers_count").and_then(|s| s.as_u64()),
                pushed_at: it
                    .get("pushed_at")
                    .and_then(|p| p.as_str())
                    .map(|p| clean_text(p, 40)),
                source: source.to_string(),
                installed: vec![],
            })
        })
        .collect())
}

/// The entries of a remote index file and how many were skipped (R3). The
/// outer shape must be `{"xherdCatalog": 1, "plugins": [...]}`, or the whole
/// source fails. An entry is skipped without a name, with a URL that is not
/// `https://`, with a ref or path git would refuse, or with an unknown kind.
pub fn parse_index(body: &[u8], source: &str) -> Result<(Vec<Entry>, usize), String> {
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("not JSON: {e}"))?;
    if v.get("xherdCatalog").and_then(|n| n.as_u64()) != Some(1) {
        return Err("not an xherd catalogue (xherdCatalog must be 1)".into());
    }
    let plugins = v
        .get("plugins")
        .and_then(|p| p.as_array())
        .ok_or("no \"plugins\" list in the catalogue")?;
    let mut entries = Vec::new();
    let mut skipped = 0;
    for p in plugins {
        match index_entry(p, source) {
            Some(e) => entries.push(e),
            None => skipped += 1,
        }
    }
    Ok((entries, skipped))
}

fn index_entry(p: &serde_json::Value, source: &str) -> Option<Entry> {
    let text = |k: &str| p.get(k).and_then(|v| v.as_str());
    let name = clean_text(text("name")?, NAME_MAX);
    let url = text("url")?.trim();
    let starts_https = url
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"));
    if name.is_empty() || !starts_https {
        return None;
    }
    let git_ref = text("ref").filter(|r| !r.is_empty());
    let path = normalise_path(text("path"));
    let parsed = crate::source::PluginSource::parse(url, git_ref, path.as_deref()).ok()?;
    let kind = match p.get("kind") {
        None | Some(serde_json::Value::Null) => None,
        Some(k) => Some(serde_json::from_value::<Kind>(k.clone()).ok()?),
    };
    // The owner is the first path segment, the host when there is none.
    let mut parts = url[8..].split('/');
    let host = parts.next().unwrap_or("");
    let owner = parts.next().filter(|s| !s.is_empty()).unwrap_or(host);
    Some(Entry {
        name,
        owner: clean_text(owner, NAME_MAX),
        description: description(text("description")),
        url: url.to_string(),
        git_ref: git_ref.map(|r| r.to_string()),
        path: parsed.path,
        kind,
        stars: None,
        pushed_at: None,
        source: source.to_string(),
        installed: vec![],
    })
}

/// The argv of one catalogue fetch (R3). The safety flags live only here:
/// `-q` first (no `.curlrc`), https only, also after redirects, at most 5
/// redirects, 1 MiB, 15 s, headers on stderr so stdout is the body, and no
/// `Authorization` header or netrc.
pub fn curl_argv(curl: &str, url: &str, github: bool) -> Vec<String> {
    let mut argv: Vec<String> = [
        curl,
        "-q",
        "-sS",
        "-L",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--max-redirs",
        "5",
        "--max-filesize",
        "1048576",
        "--max-time",
        "15",
        "-D",
        "/dev/stderr",
        "-H",
        "User-Agent: xherd",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if github {
        argv.push("-H".into());
        argv.push("Accept: application/vnd.github+json".into());
    }
    argv.push("--".into());
    argv.push(url.to_string());
    argv
}

/// Status, lower-cased headers of the last `HTTP/` block (after redirects)
/// and body of a `curl_argv` run. A missing status line is status 0.
pub fn response(stdout: &[u8], stderr: &str) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let lines: Vec<&str> = stderr.lines().map(|l| l.trim_end_matches('\r')).collect();
    let Some(start) = lines.iter().rposition(|l| l.starts_with("HTTP/")) else {
        return (0, vec![], stdout.to_vec());
    };
    let status = lines[start]
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let headers = lines[start + 1..]
        .iter()
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    (status, headers, stdout.to_vec())
}

/// The message of a GitHub answer that is not 200 (R4).
pub fn github_error(status: u16, headers: &[(String, String)]) -> String {
    if status == 403 || status == 429 {
        let reset = headers
            .iter()
            .find(|(k, _)| k == "x-ratelimit-reset")
            .and_then(|(_, v)| v.parse::<i64>().ok());
        if let Some(secs) = reset {
            let ms = secs.saturating_mul(1000);
            let day = xherd_core::usage::day(ms);
            let t = secs.rem_euclid(86_400);
            return format!(
                "GitHub rate limit reached; try again after {day}T{:02}:{:02}:{:02}Z",
                t / 3600,
                t % 3600 / 60,
                t % 60
            );
        }
        return "GitHub rate limit reached; try again later".into();
    }
    format!("HTTP {status}")
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
    fn github_answer_is_parsed() {
        let body = br#"{"total_count":3,"items":[
          {"name":"xherd-forgejo","owner":{"login":"acme"},"description":"Forgejo\u001b[31m host",
           "clone_url":"https://github.com/acme/xherd-forgejo.git","stargazers_count":12,
           "pushed_at":"2026-10-01T10:00:00Z"},
          {"name":"nodesc","owner":{"login":"b"},"description":null,
           "clone_url":"https://github.com/b/nodesc.git","stargazers_count":0,"pushed_at":null},
          {"name":"elsewhere","owner":{"login":"c"},"clone_url":"https://evil.example/c/e.git"}]}"#;
        let entries = parse_github(body, "github").unwrap();
        assert_eq!(entries.len(), 2);
        let e = &entries[0];
        assert_eq!(e.name, "xherd-forgejo");
        assert_eq!(e.owner, "acme");
        assert_eq!(e.description.as_deref(), Some("Forgejo[31m host"));
        assert_eq!(e.stars, Some(12));
        assert_eq!(e.pushed_at.as_deref(), Some("2026-10-01T10:00:00Z"));
        assert_eq!(e.source, "github");
        assert_eq!(entries[1].description, None);
        assert!(parse_github(b"{}", "github").is_err());
        assert!(parse_github(b"nope", "github").is_err());
    }

    #[test]
    fn index_is_parsed_and_bad_entries_skipped() {
        let body = br#"{"xherdCatalog":1,"plugins":[
          {"name":"forgejo","url":"https://codeberg.org/acme/xherd-forgejo","ref":"v0.3.0",
           "path":"./plugin/","description":"Forgejo host","kind":"host"},
          {"url":"https://codeberg.org/a/noname"},
          {"name":"plain","url":"http://codeberg.org/a/plain"},
          {"name":"badkind","url":"https://codeberg.org/a/k","kind":"wizard"},
          {"name":"badpath","url":"https://codeberg.org/a/p","path":"../x"},
          {"name":"badref","url":"https://codeberg.org/a/r","ref":"a..b"},
          {"name":"bare","url":"https://example.org"},
          {"name":"second","url":"https://example.org/team/second.git"}]}"#;
        let (entries, skipped) = parse_index(body, "https://e.org/c.json").unwrap();
        assert_eq!(skipped, 6);
        assert_eq!(entries.len(), 2);
        let e = &entries[0];
        assert_eq!(e.name, "forgejo");
        assert_eq!(e.owner, "acme");
        assert_eq!(e.git_ref.as_deref(), Some("v0.3.0"));
        assert_eq!(e.path.as_deref(), Some("plugin"));
        assert_eq!(e.kind, Some(Kind::Host));
        assert_eq!(e.source, "https://e.org/c.json");
        assert_eq!(entries[1].owner, "team");
    }

    #[test]
    fn index_with_a_wrong_shape_fails() {
        for bad in [
            &br#"{"plugins":[]}"#[..],
            br#"{"xherdCatalog":2,"plugins":[]}"#,
            br#"{"xherdCatalog":1}"#,
            b"[]",
            b"not json",
        ] {
            assert!(parse_index(bad, "s").is_err());
        }
        assert_eq!(
            parse_index(br#"{"xherdCatalog":1,"plugins":[]}"#, "s").unwrap(),
            (vec![], 0)
        );
    }

    #[test]
    fn curl_argv_has_the_safety_flags() {
        let argv = curl_argv("curl", "https://e.org/c.json", false);
        assert_eq!(argv[0], "curl");
        assert_eq!(argv[1], "-q");
        let has = |a: &[&str]| argv.windows(a.len()).any(|w| w == a);
        assert!(has(&["--proto", "=https"]));
        assert!(has(&["--proto-redir", "=https"]));
        assert!(has(&["--max-redirs", "5"]));
        assert!(has(&["--max-filesize", "1048576"]));
        assert!(has(&["--max-time", "15"]));
        assert!(has(&["-D", "/dev/stderr"]));
        assert_eq!(&argv[argv.len() - 2..], ["--", "https://e.org/c.json"]);
        assert!(
            !argv
                .iter()
                .any(|a| a.to_lowercase().contains("authorization"))
        );
        assert!(!argv.iter().any(|a| a.contains("vnd.github")));
        let gh = curl_argv("curl", "https://api.github.com/x", true);
        assert!(
            gh.iter()
                .any(|a| a == "Accept: application/vnd.github+json")
        );
        assert!(
            !gh.iter()
                .any(|a| a.to_lowercase().contains("authorization"))
        );
    }

    #[test]
    fn response_takes_the_last_header_block() {
        let stderr = "HTTP/2 301\r\nlocation: https://b\r\n\r\nHTTP/2 403\r\nX-RateLimit-Reset: 1764000000\r\ncontent-type: x\r\n\r\n";
        let (status, headers, body) = response(b"{}", stderr);
        assert_eq!(status, 403);
        assert_eq!(body, b"{}");
        assert!(headers.contains(&("x-ratelimit-reset".into(), "1764000000".into())));
        assert!(!headers.iter().any(|(k, _)| k == "location"));
        // curl's own error after the headers does not matter; none at all is 0.
        assert_eq!(response(b"", "curl: (6) Could not resolve host").0, 0);
        assert_eq!(response(b"x", "HTTP/1.1 200 OK\n\ncurl: (23) boom").0, 200);
    }

    #[test]
    fn github_error_names_the_reset_time() {
        let reset = vec![("x-ratelimit-reset".to_string(), "1764000000".to_string())];
        assert_eq!(
            github_error(403, &reset),
            "GitHub rate limit reached; try again after 2025-11-24T16:00:00Z"
        );
        assert!(github_error(429, &reset).contains("rate limit"));
        assert_eq!(
            github_error(429, &[]),
            "GitHub rate limit reached; try again later"
        );
        assert_eq!(github_error(404, &reset), "HTTP 404");
        assert_eq!(github_error(500, &[]), "HTTP 500");
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
