//! Code tripwires over a proposed commit's diff (pure, R-115…R-117).
//!
//! The secret tripwire (R-117): built-in patterns for common token formats,
//! private key headers, and the content of `.env`-like files; matches are
//! reported masked, the secret itself never leaves the diff. Also the diff size
//! limit (R-115). Sensitive paths (R-115) are global until repo profiles; the test
//! command waits for them; the scope fence (R-116) is a refusal in `xherd_flow::xap`.

/// One suspicious added line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub file: String,
    /// Line number in the new file.
    pub line: u32,
    pub kind: &'static str,
    /// What matched, masked: a short prefix and an ellipsis.
    pub masked: String,
}

/// Token formats: (prefix, characters allowed after it, minimum count, label).
type Rule = (&'static str, fn(char) -> bool, usize, &'static str);

fn alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}
fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}
fn upper_digit(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit()
}

const RULES: &[Rule] = &[
    ("ghp_", alnum, 36, "GitHub token"),
    ("gho_", alnum, 36, "GitHub token"),
    ("ghu_", alnum, 36, "GitHub token"),
    ("ghs_", alnum, 36, "GitHub token"),
    ("ghr_", alnum, 36, "GitHub token"),
    ("github_pat_", word, 22, "GitHub token"),
    ("glpat-", word, 20, "GitLab token"),
    ("AKIA", upper_digit, 16, "AWS access key id"),
    ("ASIA", upper_digit, 16, "AWS access key id"),
    ("xoxb-", word, 10, "Slack token"),
    ("xoxp-", word, 10, "Slack token"),
    ("xoxa-", word, 10, "Slack token"),
    ("xoxr-", word, 10, "Slack token"),
    ("xoxs-", word, 10, "Slack token"),
    ("AIza", word, 35, "Google API key"),
    ("sk-ant-", word, 20, "Anthropic API key"),
    ("sk-", word, 32, "API key"),
];

/// Does `s` contain `prefix` followed by at least `min` characters of `ok`,
/// at a token boundary (not inside a longer word)? Returns the match start.
fn token(s: &str, prefix: &str, ok: fn(char) -> bool, min: usize) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = s[from..].find(prefix) {
        let at = from + i;
        let before = s[..at].chars().next_back();
        let after = &s[at + prefix.len()..];
        let run = after.chars().take_while(|c| ok(*c)).count();
        if run >= min && !before.is_some_and(|c| c.is_ascii_alphanumeric()) {
            return Some(at);
        }
        from = at + prefix.len();
    }
    None
}

fn mask(s: &str) -> String {
    let head: String = s.chars().take(4).collect();
    format!("{head}…")
}

/// `.env`, `.env.local`, `prod.env`; not templates (`.env.example`).
fn is_env_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let template = [".example", ".sample", ".template", ".dist"]
        .iter()
        .any(|t| name.ends_with(t));
    !template && (name == ".env" || name.starts_with(".env.") || name.ends_with(".env"))
}

/// A `KEY=value` line with a non-empty, non-placeholder value.
fn env_assignment(line: &str) -> Option<&str> {
    let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
    if line.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    let value = value.trim().trim_matches(['"', '\'']);
    let key_ok = !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let placeholder = value.is_empty()
        || value.starts_with("${")
        || value.starts_with("<")
        || value.eq_ignore_ascii_case("changeme");
    (key_ok && !placeholder).then_some(key)
}

fn scan_line(file: &str, text: &str) -> Option<(&'static str, String)> {
    if let Some(i) = text.find("-----BEGIN ")
        && text[i..].contains("PRIVATE KEY")
    {
        return Some(("private key", mask(&text[i + 11..])));
    }
    for (prefix, ok, min, label) in RULES {
        if let Some(at) = token(text, prefix, *ok, *min) {
            return Some((label, mask(&text[at..])));
        }
    }
    if is_env_file(file)
        && let Some(key) = env_assignment(text)
    {
        return Some(("value in an env file", mask(key)));
    }
    None
}

/// Scan the added lines of a unified diff (`git diff --no-color`).
pub fn scan_diff(diff: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let (mut file, mut line) = (String::new(), 0u32);
    for raw in diff.lines() {
        if let Some(rest) = raw.strip_prefix("+++ ") {
            file = rest.strip_prefix("b/").unwrap_or(rest).to_string();
        } else if let Some(h) = raw.strip_prefix("@@ ") {
            // @@ -a,b +c,d @@
            line = h
                .split_whitespace()
                .find_map(|p| p.strip_prefix('+'))
                .and_then(|p| p.split(',').next())
                .and_then(|n| n.parse().ok())
                .unwrap_or(1);
        } else if raw.starts_with("---") || raw.starts_with("diff ") || raw.starts_with("index ") {
        } else if let Some(text) = raw.strip_prefix('+') {
            if let Some((kind, masked)) = scan_line(&file, text) {
                out.push(Finding {
                    file: file.clone(),
                    line,
                    kind,
                    masked,
                });
            }
            line += 1;
        } else if raw.starts_with(' ') {
            line += 1;
        }
    }
    out
}

/// One line for the gate's "held" reason; `None` when nothing was found.
pub fn summary(findings: &[Finding]) -> Option<String> {
    if findings.is_empty() {
        return None;
    }
    let shown: Vec<String> = findings
        .iter()
        .take(3)
        .map(|f| format!("{} in {}:{} ({})", f.kind, f.file, f.line, f.masked))
        .collect();
    let more = findings.len().saturating_sub(3);
    let tail = if more > 0 {
        format!(" and {more} more")
    } else {
        String::new()
    };
    Some(format!(
        "possible secret: {}{tail}; check before approving",
        shown.join(", ")
    ))
}

fn glob_at(p: &[char], s: &[char]) -> bool {
    match p.first() {
        None => s.is_empty(),
        Some('*') if p.get(1) == Some(&'*') => {
            let mut rest = &p[2..];
            // `**/` also matches no folder at all.
            if rest.first() == Some(&'/') {
                rest = &rest[1..];
                if glob_at(rest, s) {
                    return true;
                }
            }
            (0..=s.len()).any(|i| glob_at(rest, &s[i..]))
        }
        Some('*') => {
            let max = s.iter().position(|c| *c == '/').unwrap_or(s.len());
            (0..=max).any(|i| glob_at(&p[1..], &s[i..]))
        }
        Some('?') => s.first().is_some_and(|c| *c != '/') && glob_at(&p[1..], &s[1..]),
        Some(c) => s.first() == Some(c) && glob_at(&p[1..], &s[1..]),
    }
}

/// Does `path` match the glob? A pattern without `/` matches the file name
/// at any depth.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let path = path.strip_prefix("./").unwrap_or(path);
    let target = if pattern.contains('/') {
        path
    } else {
        path.rsplit('/').next().unwrap_or(path)
    };
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = target.chars().collect();
    glob_at(&p, &s)
}

/// Sensitive paths (R-115): proposed `files` matching any of the globs.
pub fn sensitive_summary(files: &[String], globs: &[String]) -> Option<String> {
    let hit: Vec<&str> = files
        .iter()
        .map(|f| f.strip_prefix("./").unwrap_or(f))
        .filter(|f| globs.iter().any(|g| glob_match(g, f)))
        .collect();
    if hit.is_empty() {
        return None;
    }
    let more = hit.len().saturating_sub(3);
    let tail = if more > 0 {
        format!(" and {more} more")
    } else {
        String::new()
    };
    Some(format!(
        "sensitive path: {}{tail}; check before approving",
        hit[..hit.len().min(3)].join(", ")
    ))
}

/// Default for R-115 `maxDiffLines`.
pub const MAX_DIFF_LINES: usize = 1000;

/// Added plus removed lines of a unified diff.
pub fn diff_lines(diff: &str) -> usize {
    diff.lines()
        .filter(|l| {
            (l.starts_with('+') && !l.starts_with("+++"))
                || (l.starts_with('-') && !l.starts_with("---"))
        })
        .count()
}

/// Reason to hold a proposal whose diff is longer than `max` lines (R-115).
pub fn size_summary(diff: &str, max: usize) -> Option<String> {
    let n = diff_lines(diff);
    (n > max).then(|| format!("large diff: {n} changed lines, limit {max}; check before approving"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(file: &str, added: &[&str]) -> String {
        let body: String = added.iter().map(|l| format!("+{l}\n")).collect();
        format!(
            "diff --git a/{file} b/{file}\nindex 1..2 100644\n--- a/{file}\n+++ b/{file}\n@@ -0,0 +10,{} @@\n{body}",
            added.len()
        )
    }

    #[test]
    fn finds_token_formats_with_line_numbers() {
        let t = format!("ghp_{}", "a1".repeat(18));
        let f = scan_diff(&diff(
            "src/a.rs",
            &["let x = 1;", &format!("let t = \"{t}\";")],
        ));
        assert_eq!(f.len(), 1);
        assert_eq!(
            (f[0].file.as_str(), f[0].line, f[0].kind),
            ("src/a.rs", 11, "GitHub token")
        );
        assert_eq!(f[0].masked, "ghp_…");
        assert!(!summary(&f).unwrap().contains(&t));
    }

    #[test]
    fn private_keys_and_aws() {
        let f = scan_diff(&diff("k.pem", &["-----BEGIN RSA PRIVATE KEY-----"]));
        assert_eq!(f[0].kind, "private key");
        let f = scan_diff(&diff("a.txt", &["id = AKIAABCDEFGHIJKLMNOP"]));
        assert_eq!(f[0].kind, "AWS access key id");
    }

    #[test]
    fn ordinary_code_is_clean() {
        let lines = [
            "let masked = \"sk-\";",
            "// task-list, ask-me, risk-averse-things-about-longnames-and-more-words",
            "fn skip_ghp_prefix() {}",
            "-----BEGIN CERTIFICATE-----",
        ];
        assert!(scan_diff(&diff("src/b.rs", &lines)).is_empty());
    }

    #[test]
    fn word_boundary_avoids_inner_matches() {
        // "task-" contains "sk-" but sits inside a word.
        assert!(scan_diff(&diff("a.md", &["task-0123456789012345678901234567890123"])).is_empty());
        let f = scan_diff(&diff(
            "a.md",
            &["key: sk-0123456789012345678901234567890123"],
        ));
        assert_eq!(f[0].kind, "API key");
    }

    #[test]
    fn env_files_flag_values_not_templates() {
        let f = scan_diff(&diff(
            ".env",
            &["# note", "DB_URL=postgres://u:p@h/db", "EMPTY=", "X=${Y}"],
        ));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "value in an env file");
        assert!(scan_diff(&diff(".env.example", &["DB_URL=postgres://u:p@h/db"])).is_empty());
    }

    #[test]
    fn summary_caps_and_counts() {
        let lines: Vec<String> = (0..5)
            .map(|i| format!("-----BEGIN PRIVATE KEY----- {i}"))
            .collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let s = summary(&scan_diff(&diff("k", &refs))).unwrap();
        assert!(s.contains("and 2 more"), "{s}");
        assert_eq!(summary(&[]), None);
    }

    #[test]
    fn globs() {
        assert!(glob_match("*.pem", "keys/a.pem"));
        assert!(glob_match(
            ".github/workflows/*",
            "./.github/workflows/ci.yml"
        ));
        assert!(!glob_match(
            ".github/workflows/*",
            ".github/workflows/x/ci.yml"
        ));
        assert!(glob_match("migrations/**", "migrations/2024/a.sql"));
        assert!(glob_match("**/secrets.yml", "secrets.yml"));
        assert!(glob_match("**/secrets.yml", "a/b/secrets.yml"));
        assert!(!glob_match("src/*.rs", "src/a/b.rs"));
        let f = vec!["./a.rs".to_string(), "infra/prod.tf".to_string()];
        let g = vec!["infra/**".to_string()];
        assert!(sensitive_summary(&f, &g).unwrap().contains("infra/prod.tf"));
        assert!(sensitive_summary(&f, &[]).is_none());
    }

    #[test]
    fn size_counts_changed_lines_only() {
        let d = diff("a.rs", &["x"; 5]);
        assert_eq!(diff_lines(&d), 5);
        assert!(size_summary(&d, 5).is_none());
        assert!(size_summary(&d, 4).unwrap().contains("5 changed lines"));
    }
}
