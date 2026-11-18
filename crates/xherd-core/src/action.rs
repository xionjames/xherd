//! Job actions: named commands a job registers, the user runs (R1, R3, R14,
//! R18). Pure types and checks; the actor changes `Job.actions` through them.

use serde::{Deserialize, Serialize};

use crate::hash::content_sha256;
use crate::xap::{Refusal, refuse};

pub const MAX_ID: usize = 32;
pub const MAX_LABEL: usize = 60;
pub const MAX_DESCRIPTION: usize = 300;
pub const MAX_RUN: usize = 500;
pub const MAX_CWD: usize = 200;

/// Whether an action keeps running (a dev server) or ends (a test run).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionKind {
    Service,
    #[default]
    Task,
}

/// Where an action came from: registered by the job, or copied from the
/// repository profile (R19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionSource {
    #[default]
    Job,
    Profile,
}

/// What an action is (also what a profile stores, R18). `cwd` is relative to
/// the worktree root, `""` meaning the root.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct ActionDef {
    pub id: String,
    pub label: String,
    pub description: String,
    pub run: String,
    pub cwd: String,
    pub kind: ActionKind,
    pub icon: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionState {
    Running,
    Exited,
    Stopped,
    Lost,
}

/// The latest run of an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionRun {
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    pub state: ActionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
}

/// An action as a job holds it. `approved` is the `approval_hash` the user
/// allowed (R14); `succeeded` is set once a run counted as a success (R23).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobAction {
    pub def: ActionDef,
    #[serde(default)]
    pub source: ActionSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<ActionRun>,
    #[serde(default)]
    pub succeeded: bool,
}

/// `sha256(run + "\0" + cwd)`: what the user approves (R14). Label,
/// description, kind and icon are not part of it.
pub fn approval_hash(def: &ActionDef) -> String {
    let mut b = Vec::with_capacity(def.run.len() + def.cwd.len() + 1);
    b.extend_from_slice(def.run.as_bytes());
    b.push(0);
    b.extend_from_slice(def.cwd.as_bytes());
    content_sha256(&b)
}

fn plain(s: &str) -> bool {
    !s.chars().any(char::is_control)
}

fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= MAX_ID
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

fn valid_icon(icon: &str) -> bool {
    !icon.is_empty()
        && icon.len() <= MAX_ID
        && icon
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// Lexical clean-up of a relative folder: no absolute path, `..` may not
/// climb above the root, `.` and empty segments vanish, `/` joins (R3, R18).
/// Symlinks are checked later against the real worktree.
pub fn clean_cwd(cwd: &str) -> Result<String, Refusal> {
    let cwd = cwd.trim();
    if cwd.len() > MAX_CWD || !plain(cwd) {
        return Err(refuse("usage", "cwd: a short relative folder"));
    }
    if cwd.starts_with('/') || cwd.starts_with('\\') || cwd.contains('\\') || cwd.contains(':') {
        return Err(refuse(
            "path_outside_workspace",
            "cwd must be relative to the worktree",
        ));
    }
    let mut parts: Vec<&str> = Vec::new();
    for seg in cwd.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(refuse("path_outside_workspace", "cwd leaves the worktree"));
                }
            }
            s => parts.push(s),
        }
    }
    Ok(parts.join("/"))
}

/// Trim and validate a definition (R1): slug id, short plain label and
/// description, one-line command, relative cwd, icon name.
pub fn check_def(d: &mut ActionDef) -> Result<(), Refusal> {
    d.id = d.id.trim().to_string();
    d.label = d.label.trim().to_string();
    d.description = d.description.trim().to_string();
    d.run = d.run.trim().to_string();
    d.icon = d.icon.trim().to_string();
    if !valid_id(&d.id) {
        return Err(refuse(
            "usage",
            format!("id: a slug of a-z, 0-9 and -, up to {MAX_ID} characters"),
        ));
    }
    if d.label.is_empty() || d.label.chars().count() > MAX_LABEL || !plain(&d.label) {
        return Err(refuse(
            "usage",
            format!("label: 1 to {MAX_LABEL} characters of plain text"),
        ));
    }
    if d.description.chars().count() > MAX_DESCRIPTION || !plain(&d.description) {
        return Err(refuse(
            "usage",
            format!("description: up to {MAX_DESCRIPTION} characters of plain text"),
        ));
    }
    if d.run.is_empty() || d.run.len() > MAX_RUN || !plain(&d.run) {
        return Err(refuse(
            "usage",
            format!("run: one line, up to {MAX_RUN} bytes"),
        ));
    }
    if !d.icon.is_empty() && !valid_icon(&d.icon) {
        return Err(refuse("usage", "icon: a name of a-z, 0-9 and -"));
    }
    d.cwd = clean_cwd(&d.cwd)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn def() -> ActionDef {
        ActionDef {
            id: "dev".into(),
            label: "Dev server".into(),
            run: "npm run dev".into(),
            ..ActionDef::default()
        }
    }

    fn code(d: ActionDef) -> &'static str {
        let mut d = d;
        check_def(&mut d).unwrap_err().code
    }

    #[test]
    fn a_good_definition_is_kept_and_cleaned() {
        let mut d = def();
        d.id = " dev ".into();
        d.cwd = "./web//app/../".into();
        d.icon = "server".into();
        check_def(&mut d).unwrap();
        assert_eq!(d.id, "dev");
        assert_eq!(d.cwd, "web");
        assert_eq!(d.kind, ActionKind::Task);
    }

    #[test]
    fn ids_are_slugs() {
        for bad in ["", "-a", "A", "a b", "a_b", "é", &"a".repeat(33)] {
            let mut d = def();
            d.id = bad.into();
            assert_eq!(code(d), "usage", "{bad:?}");
        }
        for good in ["a", "0", "a-b-9", &"a".repeat(32)] {
            let mut d = def();
            d.id = good.into();
            check_def(&mut d).unwrap();
        }
    }

    #[test]
    fn text_limits() {
        let mut d = def();
        d.label = "x".repeat(61);
        assert_eq!(code(d), "usage");
        let mut d = def();
        d.label = "".into();
        assert_eq!(code(d), "usage");
        let mut d = def();
        d.description = "x".repeat(301);
        assert_eq!(code(d), "usage");
        let mut d = def();
        d.label = "a\u{7}b".into();
        assert_eq!(code(d), "usage");
        let mut d = def();
        d.run = "a\nb".into();
        assert_eq!(code(d), "usage");
        let mut d = def();
        d.run = "x".repeat(501);
        assert_eq!(code(d), "usage");
        let mut d = def();
        d.run = " ".into();
        assert_eq!(code(d), "usage");
        let mut d = def();
        d.icon = "Server!".into();
        assert_eq!(code(d), "usage");
        let mut d = def();
        d.run = "x".repeat(500);
        d.label = "é".repeat(60);
        check_def(&mut d).unwrap();
    }

    #[test]
    fn cwd_stays_inside() {
        assert_eq!(clean_cwd("").unwrap(), "");
        assert_eq!(clean_cwd(".").unwrap(), "");
        assert_eq!(clean_cwd("a/./b/").unwrap(), "a/b");
        assert_eq!(clean_cwd("a/../b").unwrap(), "b");
        for bad in ["..", "../x", "a/../..", "/etc", "C:x", "a\\b"] {
            assert_eq!(
                clean_cwd(bad).unwrap_err().code,
                "path_outside_workspace",
                "{bad:?}"
            );
        }
        assert_eq!(clean_cwd("a\0b").unwrap_err().code, "usage");
    }

    #[test]
    fn approval_covers_run_and_cwd_only() {
        let a = def();
        let mut b = def();
        b.label = "Other".into();
        b.description = "d".into();
        b.kind = ActionKind::Service;
        b.icon = "x".into();
        assert_eq!(approval_hash(&a), approval_hash(&b));
        b.run = "npm start".into();
        assert_ne!(approval_hash(&a), approval_hash(&b));
        let mut c = def();
        c.cwd = "web".into();
        assert_ne!(approval_hash(&a), approval_hash(&c));
        // "ab" + "" and "a" + "b" must not collide.
        let mut d1 = def();
        d1.run = "ab".into();
        let mut d2 = def();
        d2.run = "a".into();
        d2.cwd = "b".into();
        assert_ne!(approval_hash(&d1), approval_hash(&d2));
    }

    #[test]
    fn wire_format() {
        let a = JobAction {
            def: ActionDef {
                kind: ActionKind::Service,
                cwd: "web".into(),
                ..def()
            },
            source: ActionSource::Profile,
            approved: Some("h".into()),
            last: Some(ActionRun {
                started_at: 5,
                ended_at: None,
                state: ActionState::Running,
                exit: None,
            }),
            succeeded: true,
        };
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["def"]["kind"], "service");
        assert_eq!(v["source"], "profile");
        assert_eq!(v["last"]["startedAt"], 5);
        assert_eq!(v["last"]["state"], "running");
        assert!(v["last"].get("endedAt").is_none());
        let back: JobAction = serde_json::from_value(v).unwrap();
        assert_eq!(back, a);
        assert!(
            serde_json::from_value::<ActionDef>(serde_json::json!({"id": "a", "zzz": 1})).is_err()
        );
    }

    proptest! {
        #[test]
        fn clean_cwd_never_escapes(s in "[a-c./]{0,24}") {
            if let Ok(c) = clean_cwd(&s) {
                prop_assert!(!c.starts_with('/'));
                prop_assert!(c.split('/').all(|p| p != ".." && p != "."));
                prop_assert_eq!(clean_cwd(&c).unwrap(), c);
            }
        }
    }
}
