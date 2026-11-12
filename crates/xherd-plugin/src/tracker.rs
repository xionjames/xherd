//! Issue trackers: what a `tracker` plugin declares in `data.tracker`.
//! Today that is link rules: a pattern for text in the UI (a ticket key, an
//! `owner/repo#12` reference) and the URL it becomes. xherd's core never
//! calls a tracker (spec N1); links only point the reader at it.
//!
//! The patterns run in the browser, over agent-written text, so they are
//! checked when the manifest is read: they must compile with the linear-time
//! `regex` crate (no backreferences or lookaround), stay short, and not
//! repeat a repeated group (`(a+)+`), the shape of catastrophic backtracking
//! in a browser's regex engine. The URL is a template: `{baseUrl}` (the
//! plugin's `baseUrl` setting, https only) and `{0}`…`{9}` (the match and its
//! groups, percent-encoded where they are used).

use regex::Regex;
use serde::Deserialize;

use crate::Manifest;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tracker {
    #[serde(default)]
    pub links: Vec<Link>,
    /// Which tickets the tracker can read: a regular expression with one
    /// group whose match is the issue reference handed to the plugin
    /// (`^(?:GH-)?(\d+)$` turns `GH-12` or `12` into `12`). Without it the
    /// tracker only makes links.
    #[serde(default)]
    pub key: Option<String>,
    /// Git host names the tracker serves (`github.com`, `*gitlab*`): it is
    /// asked about tickets of repositories on them before any other tracker.
    #[serde(default)]
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Link {
    /// A regular expression (JavaScript and Rust agree on this subset).
    pub pattern: String,
    /// `https://…` or starting with `{baseUrl}`.
    pub url: String,
}

const MAX_PATTERN: usize = 200;

impl Tracker {
    /// `None` when the plugin declares no `data.tracker`.
    pub fn of(m: &Manifest) -> Result<Option<Tracker>, String> {
        let Some(v) = m.data.get("tracker") else {
            return Ok(None);
        };
        let t: Tracker =
            serde_json::from_value(v.clone()).map_err(|e| format!("tracker {}: {e}", m.name))?;
        for l in &t.links {
            l.validate()
                .map_err(|e| format!("tracker {}: {e}", m.name))?;
        }
        t.validate_key()
            .map_err(|e| format!("tracker {}: {e}", m.name))?;
        for h in &t.hosts {
            if h.is_empty()
                || !h
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '*'))
            {
                return Err(format!("tracker {}: {h:?} is not a host pattern", m.name));
            }
        }
        Ok(Some(t))
    }

    fn validate_key(&self) -> Result<(), String> {
        let Some(k) = &self.key else { return Ok(()) };
        let re = Regex::new(k).map_err(|e| format!("key {k:?}: {e}"))?;
        if k.len() > 100 || re.captures_len() != 2 {
            return Err(format!(
                "key {k:?} needs one group and at most 100 characters"
            ));
        }
        Ok(())
    }

    /// The issue reference `ticket` stands for (the key's group), if this
    /// tracker reads it. The reference is a plain name: it ends up in a
    /// command line.
    pub fn issue_ref(&self, ticket: &str) -> Option<String> {
        let re = Regex::new(self.key.as_ref()?).ok()?;
        let r = re.captures(ticket)?.get(1)?.as_str();
        (!r.is_empty()
            && r.len() <= 40
            && r.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .then(|| r.to_string())
    }

    /// The tracker serves repositories on `host`.
    pub fn serves(&self, host: &str) -> bool {
        self.hosts.iter().any(|p| crate::git::host_matches(p, host))
    }
}

impl Link {
    fn validate(&self) -> Result<(), String> {
        if self.pattern.is_empty() || self.pattern.len() > MAX_PATTERN {
            return Err(format!("pattern must be 1 to {MAX_PATTERN} characters"));
        }
        if self.pattern.chars().any(char::is_control) || self.url.chars().any(char::is_control) {
            return Err("a control character".into());
        }
        let re =
            Regex::new(&self.pattern).map_err(|e| format!("pattern {:?}: {e}", self.pattern))?;
        if repeats_a_repeat(&self.pattern) {
            return Err(format!(
                "pattern {:?} repeats a repeated group",
                self.pattern
            ));
        }
        if !(self.url.starts_with("https://") || self.url.starts_with("{baseUrl}")) {
            return Err(format!(
                "url {:?} must be https or start with {{baseUrl}}",
                self.url
            ));
        }
        // Only {baseUrl} and groups the pattern has.
        let mut rest = self.url.as_str();
        while let Some(i) = rest.find('{') {
            let end = rest[i..].find('}').ok_or("an unclosed { in the url")? + i;
            let name = &rest[i + 1..end];
            let ok = name == "baseUrl"
                || name
                    .parse::<usize>()
                    .is_ok_and(|n| n <= 9 && n < re.captures_len());
            if !ok {
                return Err(format!(
                    "url {:?} uses {{{name}}}, which is not a group",
                    self.url
                ));
            }
            rest = &rest[end + 1..];
        }
        Ok(())
    }
}

/// A group that holds a quantifier and is itself quantified: `(a+)+`,
/// `(\w*)*`, `(a|b+){2,}`.
fn repeats_a_repeat(p: &str) -> bool {
    let b = p.as_bytes();
    let (mut stack, mut i, mut class) = (Vec::<bool>::new(), 0, false);
    let quant = |c: u8| matches!(c, b'+' | b'*' | b'{');
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'[' if !class => class = true,
            b']' if class => class = false,
            b'(' if !class => stack.push(false),
            b')' if !class => {
                let inner = stack.pop().unwrap_or(false);
                if inner && b.get(i + 1).is_some_and(|c| quant(*c)) {
                    return true;
                }
                // A group that is repeated, or holds a repeat, makes its parent one.
                if let Some(top) = stack.last_mut() {
                    *top |= inner || b.get(i + 1).is_some_and(|c| quant(*c));
                }
            }
            c if !class && quant(c) => {
                if let Some(top) = stack.last_mut() {
                    *top = true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest(tracker: serde_json::Value) -> Manifest {
        Manifest::parse(
            &json!({"name": "t", "kind": "tracker", "version": "0.1.0", "data": {"tracker": tracker}})
                .to_string(),
        )
        .unwrap()
    }

    #[test]
    fn reads_link_rules() {
        let t = Tracker::of(&manifest(json!({"links": [
            {"pattern": r"\b[A-Z][A-Z0-9]+-\d+\b", "url": "{baseUrl}/browse/{0}"},
            {"pattern": r"\b([\w.-]+/[\w.-]+)#(\d+)\b", "url": "https://github.com/{1}/issues/{2}"}
        ]})))
        .unwrap()
        .unwrap();
        assert_eq!(t.links.len(), 2);
        let none = Manifest::parse(
            &json!({"name": "t", "kind": "tracker", "version": "0.1.0"}).to_string(),
        )
        .unwrap();
        assert_eq!(Tracker::of(&none).unwrap(), None);
    }

    #[test]
    fn refuses_unsafe_or_broken_links() {
        for bad in [
            json!({"links": [{"pattern": "(a+)+", "url": "https://x/{0}"}]}),
            json!({"links": [{"pattern": r"(\w*)*b", "url": "https://x/{0}"}]}),
            json!({"links": [{"pattern": "(a|b+){2,}", "url": "https://x/{0}"}]}),
            json!({"links": [{"pattern": "(?=a)b", "url": "https://x/{0}"}]}),
            json!({"links": [{"pattern": r"(a)\1", "url": "https://x/{0}"}]}),
            json!({"links": [{"pattern": "a".repeat(201), "url": "https://x/{0}"}]}),
            json!({"links": [{"pattern": "a", "url": "http://x/{0}"}]}),
            json!({"links": [{"pattern": "a", "url": "javascript:alert(1)"}]}),
            json!({"links": [{"pattern": "a", "url": "https://x/{1}"}]}),
            json!({"links": [{"pattern": "a", "url": "https://x/{nope}"}]}),
            json!({"links": [{"pattern": "a", "url": "https://x/{0"}]}),
            json!({"links": [{"pattern": "a", "url": "https://x/\n"}]}),
            json!({"unknown": 1}),
        ] {
            assert!(Tracker::of(&manifest(bad.clone())).is_err(), "{bad}");
        }
        // Plain repeats and a quantified group with nothing repeated inside are fine.
        for ok in [r"[A-Z]+-\d+", r"(ab)+", r"(\d+)\.(\d+)", r"[+*]+-\d+"] {
            let m = manifest(json!({"links": [{"pattern": ok, "url": "https://x/{0}"}]}));
            assert!(Tracker::of(&m).is_ok(), "{ok}");
        }
    }

    #[test]
    fn the_built_in_trackers_are_valid() {
        let found: Vec<_> = crate::builtins()
            .into_iter()
            .filter(|m| m.kind == crate::Kind::Tracker)
            .map(|m| {
                (
                    m.name.clone(),
                    Tracker::of(&m).unwrap().unwrap().links.len(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                ("jira".to_string(), 1),
                ("github-issues".to_string(), 1),
                ("gitlab-issues".to_string(), 1)
            ]
        );
    }

    #[test]
    fn keys_pick_the_issue_reference() {
        let t = Tracker::of(&manifest(json!({
            "key": r"^(?:GH-)?(\d+)$", "hosts": ["github.com", "*gitlab*"]
        })))
        .unwrap()
        .unwrap();
        assert_eq!(t.issue_ref("GH-12").as_deref(), Some("12"));
        assert_eq!(t.issue_ref("12").as_deref(), Some("12"));
        assert_eq!(t.issue_ref("PAY-12"), None);
        assert!(t.serves("github.com") && t.serves("my-gitlab.example.com") && !t.serves("x.org"));
        // A tracker without a key reads nothing; a bad key or host is refused.
        let none = Tracker::of(&manifest(json!({"links": []})))
            .unwrap()
            .unwrap();
        assert_eq!(none.issue_ref("12"), None);
        for bad in [
            json!({"key": r"^\d+$"}),
            json!({"key": "(a)(b)"}),
            json!({"key": "("}),
            json!({"hosts": ["bad host"]}),
        ] {
            assert!(Tracker::of(&manifest(bad.clone())).is_err(), "{bad}");
        }
        // The reference is a plain name, whatever the key lets through.
        let wide = Tracker::of(&manifest(json!({"key": "^(.*)$"})))
            .unwrap()
            .unwrap();
        assert_eq!(wide.issue_ref("a b; rm"), None);
    }
}
