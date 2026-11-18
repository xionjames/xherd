//! Job actions: named commands a job registers, the user runs (R1, R3, R14,
//! R18). Pure types and checks; the actor changes `Job.actions` through them.

use serde::{Deserialize, Serialize};

use crate::hash::content_sha256;
use crate::job::Job;
use crate::status::Status;
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

/// A service counts as a success once it ran this long (R23).
pub const SERVICE_OK_MS: i64 = 10_000;

/// What `may_run` decided: the action to run and whether this request's
/// approval has to be stored with it first (R14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPlan {
    pub def: ActionDef,
    pub store_approval: Option<String>,
}

/// The tmux session of an action (R9).
pub fn session_name(job_name: &str, id: &str) -> String {
    format!("xherd-{job_name}-act-{id}")
}

/// Whether any action of the job runs (R12a).
pub fn running(job: &Job) -> bool {
    job.actions
        .iter()
        .any(|a| matches!(&a.last, Some(r) if r.state == ActionState::Running))
}

fn is_running(a: &JobAction) -> bool {
    matches!(&a.last, Some(r) if r.state == ActionState::Running)
}

/// A job-registered action; the same id replaces it (R2). The job's source
/// becomes `Job` (R21); the approval and the success mark are kept only
/// while `run` and `cwd` are unchanged. Returns whether anything changed.
pub fn add(job: &mut Job, mut def: ActionDef, max: usize) -> Result<bool, Refusal> {
    check_def(&mut def)?;
    let Some(i) = job.actions.iter().position(|a| a.def.id == def.id) else {
        if job.actions.len() >= max {
            return Err(refuse(
                "action_limit",
                format!("a job holds at most {max} actions"),
            ));
        }
        job.actions.push(JobAction {
            def,
            source: ActionSource::Job,
            approved: None,
            last: None,
            succeeded: false,
        });
        return Ok(true);
    };
    let a = &mut job.actions[i];
    let same_command = approval_hash(&a.def) == approval_hash(&def);
    if same_command && a.def == def && a.source == ActionSource::Job {
        return Ok(false);
    }
    if !same_command {
        a.approved = None;
        a.succeeded = false;
    }
    a.def = def;
    a.source = ActionSource::Job;
    Ok(true)
}

/// Remove an action and hand it back (the caller stops it when it runs).
pub fn remove(job: &mut Job, id: &str) -> Result<JobAction, Refusal> {
    match job.actions.iter().position(|a| a.def.id == id) {
        Some(i) => Ok(job.actions.remove(i)),
        None => Err(unknown(id)),
    }
}

fn unknown(id: &str) -> Refusal {
    refuse("action_unknown", format!("no action `{id}`"))
}

fn find<'a>(job: &'a mut Job, id: &str) -> Result<&'a mut JobAction, Refusal> {
    job.actions
        .iter_mut()
        .find(|a| a.def.id == id)
        .ok_or_else(|| unknown(id))
}

/// May the user run this action now? (R11, R14.) A job in `error` keeps its
/// actions and may run them, like a parked one.
pub fn may_run(job: &Job, id: &str, approve: bool) -> Result<RunPlan, Refusal> {
    let a = job
        .actions
        .iter()
        .find(|a| a.def.id == id)
        .ok_or_else(|| unknown(id))?;
    if matches!(
        job.status,
        Status::Queued | Status::Paused | Status::Cancelled | Status::Done
    ) {
        return Err(refuse(
            "not_allowed",
            format!("a {} job cannot run actions", job.status.as_str()),
        ));
    }
    if job.worktree_removed_at.is_some() {
        return Err(refuse("worktree_removed", "the worktree was removed"));
    }
    if job.workdir.is_none() {
        return Err(refuse("no_workspace", "the job has no worktree"));
    }
    if is_running(a) {
        return Err(refuse("action_running", format!("`{id}` already runs")));
    }
    let hash = approval_hash(&a.def);
    if a.approved.as_deref() == Some(hash.as_str()) {
        return Ok(RunPlan {
            def: a.def.clone(),
            store_approval: None,
        });
    }
    if !approve {
        return Err(refuse(
            "action_not_approved",
            format!("`{id}` needs the user's approval of its command"),
        ));
    }
    Ok(RunPlan {
        def: a.def.clone(),
        store_approval: Some(hash),
    })
}

/// Approve `hash` for the action when it still matches its command (save to
/// the profile, R22). False when the action changed or is gone.
pub fn approve(job: &mut Job, id: &str, hash: &str) -> bool {
    match find(job, id) {
        Ok(a) if approval_hash(&a.def) == hash => {
            a.approved = Some(hash.to_string());
            true
        }
        _ => false,
    }
}

/// A run started; `approval` is the hash to store (from `RunPlan`).
pub fn on_started(job: &mut Job, id: &str, now: i64, approval: Option<String>) {
    if let Ok(a) = find(job, id) {
        if approval.is_some() {
            a.approved = approval;
        }
        a.last = Some(ActionRun {
            started_at: now,
            ended_at: None,
            state: ActionState::Running,
            exit: None,
        });
    }
}

fn ran_long_enough(r: &ActionRun, now: i64) -> bool {
    now.saturating_sub(r.started_at) >= SERVICE_OK_MS
}

/// A pane check of a running service: record the success once it has lived
/// long enough (R23). True when it changed.
pub fn on_alive(job: &mut Job, id: &str, now: i64) -> bool {
    match find(job, id) {
        Ok(a) if a.def.kind == ActionKind::Service && !a.succeeded => match &a.last {
            Some(r) if r.state == ActionState::Running && ran_long_enough(r, now) => {
                a.succeeded = true;
                true
            }
            _ => false,
        },
        _ => false,
    }
}

fn end(job: &mut Job, id: &str, now: i64, state: ActionState, exit: Option<i32>) {
    let Ok(a) = find(job, id) else { return };
    let Some(r) = a.last.as_mut() else { return };
    if r.state != ActionState::Running {
        return;
    }
    r.state = state;
    r.ended_at = Some(now);
    r.exit = exit;
    let ok = match (a.def.kind, state) {
        (ActionKind::Task, ActionState::Exited) => exit == Some(0),
        (ActionKind::Service, ActionState::Exited | ActionState::Stopped) => {
            ran_long_enough(r, now)
        }
        _ => false,
    };
    if ok {
        a.succeeded = true;
    }
}

/// The command ended by itself (its exit status is known).
pub fn on_exited(job: &mut Job, id: &str, now: i64, exit: i32) {
    end(job, id, now, ActionState::Exited, Some(exit));
}

/// The user stopped it (or removal, pause, cancel…).
pub fn on_stopped(job: &mut Job, id: &str, now: i64, exit: Option<i32>) {
    end(job, id, now, ActionState::Stopped, exit);
}

/// Its session vanished (daemon restart, tmux killed): no exit status.
pub fn on_lost(job: &mut Job, id: &str, now: i64) {
    end(job, id, now, ActionState::Lost, None);
}

/// The ids to offer at the end of a job (R23): succeeded at least once and
/// not on the profile with the same command. `profile` is `None` when the
/// job has no profile (everything succeeded is offered).
pub fn save_offer(job: &Job, profile: Option<&[ActionDef]>) -> Vec<String> {
    job.actions
        .iter()
        .filter(|a| a.succeeded)
        .filter(|a| {
            let h = approval_hash(&a.def);
            !profile
                .unwrap_or_default()
                .iter()
                .any(|p| p.id == a.def.id && approval_hash(p) == h)
        })
        .map(|a| a.def.id.clone())
        .collect()
}

/// A profile's action as a new job holds it: approved (R19).
pub fn from_profile(def: &ActionDef) -> JobAction {
    JobAction {
        approved: Some(approval_hash(def)),
        def: def.clone(),
        source: ActionSource::Profile,
        last: None,
        succeeded: false,
    }
}

fn state_text(a: &JobAction) -> String {
    match &a.last {
        None => "never run".into(),
        Some(r) => match (r.state, r.exit) {
            (ActionState::Running, _) => "running".into(),
            (ActionState::Exited, Some(c)) => format!("exited {c}"),
            (ActionState::Exited, None) => "exited".into(),
            (ActionState::Stopped, _) => "stopped".into(),
            (ActionState::Lost, _) => "lost".into(),
        },
    }
}

/// `action ls` (R5): one line per action.
pub fn ls_text(job: &Job) -> String {
    if job.actions.is_empty() {
        return "No actions registered.".into();
    }
    job.actions
        .iter()
        .map(|a| {
            let d = &a.def;
            let kind = match d.kind {
                ActionKind::Service => "service",
                ActionKind::Task => "task",
            };
            let source = match a.source {
                ActionSource::Job => "job",
                ActionSource::Profile => "profile",
            };
            let cwd = if d.cwd.is_empty() { "." } else { &d.cwd };
            format!(
                "{} [{kind}] {} | run: {} | cwd: {cwd} | source: {source} | {}",
                d.id,
                d.label,
                d.run,
                state_text(a)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
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

    fn job() -> Job {
        let mut j = Job::new("feat", 1);
        j.status = Status::Idle;
        j.workdir = Some("/w/feat".into());
        j
    }

    fn with(j: &mut Job, id: &str, kind: ActionKind) {
        let d = ActionDef {
            id: id.into(),
            kind,
            ..def()
        };
        add(j, d, 12).unwrap();
    }

    #[test]
    fn add_replace_and_limit() {
        let mut j = job();
        assert!(add(&mut j, def(), 2).unwrap());
        assert_eq!(j.actions[0].source, ActionSource::Job);
        assert!(j.actions[0].approved.is_none());
        // Same definition again: nothing changed.
        assert!(!add(&mut j, def(), 2).unwrap());
        let mut other = def();
        other.id = "t".into();
        add(&mut j, other.clone(), 2).unwrap();
        other.id = "u".into();
        assert_eq!(add(&mut j, other, 2).unwrap_err().code, "action_limit");
        // Replacing at the limit is fine.
        let mut d = def();
        d.label = "New".into();
        assert!(add(&mut j, d, 2).unwrap());
        assert_eq!(j.actions.len(), 2);
        let mut bad = def();
        bad.id = "A".into();
        assert_eq!(add(&mut j, bad, 2).unwrap_err().code, "usage");
    }

    #[test]
    fn replace_keeps_approval_only_for_the_same_command() {
        let mut j = job();
        add(&mut j, def(), 12).unwrap();
        j.actions[0].approved = Some(approval_hash(&j.actions[0].def));
        j.actions[0].succeeded = true;
        let mut d = def();
        d.label = "Renamed".into();
        d.kind = ActionKind::Service;
        add(&mut j, d, 12).unwrap();
        assert!(j.actions[0].approved.is_some() && j.actions[0].succeeded);
        let mut d = def();
        d.cwd = "web".into();
        add(&mut j, d, 12).unwrap();
        assert!(j.actions[0].approved.is_none() && !j.actions[0].succeeded);
        // A profile action overridden for this job only (R21).
        let p = from_profile(&def());
        j.actions = vec![p];
        let mut d = def();
        d.description = "mine".into();
        assert!(add(&mut j, d, 12).unwrap());
        assert_eq!(j.actions[0].source, ActionSource::Job);
        assert!(j.actions[0].approved.is_some());
        let mut d = def();
        d.run = "npm start".into();
        add(&mut j, d, 12).unwrap();
        assert!(j.actions[0].approved.is_none());
    }

    #[test]
    fn remove_returns_the_action() {
        let mut j = job();
        with(&mut j, "a", ActionKind::Task);
        assert_eq!(remove(&mut j, "a").unwrap().def.id, "a");
        assert_eq!(remove(&mut j, "a").unwrap_err().code, "action_unknown");
    }

    #[test]
    fn may_run_checks_in_order() {
        let mut j = job();
        assert_eq!(may_run(&j, "dev", true).unwrap_err().code, "action_unknown");
        with(&mut j, "dev", ActionKind::Service);
        assert_eq!(
            may_run(&j, "dev", false).unwrap_err().code,
            "action_not_approved"
        );
        let plan = may_run(&j, "dev", true).unwrap();
        assert_eq!(plan.store_approval, Some(approval_hash(&plan.def)));
        for s in [
            Status::Queued,
            Status::Paused,
            Status::Cancelled,
            Status::Done,
        ] {
            j.status = s;
            assert_eq!(may_run(&j, "dev", true).unwrap_err().code, "not_allowed");
        }
        for s in [
            Status::Parked,
            Status::Watching,
            Status::Error,
            Status::Running,
        ] {
            j.status = s;
            may_run(&j, "dev", true).unwrap();
        }
        j.workdir = None;
        assert_eq!(may_run(&j, "dev", true).unwrap_err().code, "no_workspace");
        j.workdir = Some("/w".into());
        j.worktree_removed_at = Some(5);
        assert_eq!(
            may_run(&j, "dev", true).unwrap_err().code,
            "worktree_removed"
        );
    }

    #[test]
    fn approved_commands_run_again_without_asking_until_they_change() {
        let mut j = job();
        with(&mut j, "dev", ActionKind::Task);
        let plan = may_run(&j, "dev", true).unwrap();
        on_started(&mut j, "dev", 100, plan.store_approval);
        assert!(running(&j));
        assert_eq!(may_run(&j, "dev", true).unwrap_err().code, "action_running");
        on_exited(&mut j, "dev", 200, 0);
        assert!(!running(&j));
        let again = may_run(&j, "dev", false).unwrap();
        assert!(again.store_approval.is_none());
        let mut d = def();
        d.run = "make".into();
        add(&mut j, d, 12).unwrap();
        assert_eq!(
            may_run(&j, "dev", false).unwrap_err().code,
            "action_not_approved"
        );
        // Approving by hash only works while the command is the same.
        let h = approval_hash(&j.actions[0].def);
        assert!(approve(&mut j, "dev", &h));
        assert!(!approve(&mut j, "dev", "stale"));
        assert!(!approve(&mut j, "nope", &h));
        may_run(&j, "dev", false).unwrap();
    }

    #[test]
    fn run_results() {
        let mut j = job();
        with(&mut j, "t", ActionKind::Task);
        with(&mut j, "s", ActionKind::Service);
        on_started(&mut j, "t", 0, None);
        on_exited(&mut j, "t", 5, 2);
        let r = j.actions[0].last.clone().unwrap();
        assert_eq!(
            (r.state, r.exit, r.ended_at),
            (ActionState::Exited, Some(2), Some(5))
        );
        assert!(!j.actions[0].succeeded);
        on_started(&mut j, "t", 10, None);
        on_exited(&mut j, "t", 15, 0);
        assert!(j.actions[0].succeeded);
        // A later failure does not take the success back.
        on_started(&mut j, "t", 20, None);
        on_exited(&mut j, "t", 25, 1);
        assert!(j.actions[0].succeeded);
        // A stopped task is not a success.
        let mut j2 = job();
        with(&mut j2, "t", ActionKind::Task);
        on_started(&mut j2, "t", 0, None);
        on_stopped(&mut j2, "t", 20_000, Some(0));
        assert!(!j2.actions[0].succeeded);
        // Services: alive for 10 s.
        on_started(&mut j, "s", 1_000, None);
        assert!(!on_alive(&mut j, "s", 10_999));
        assert!(on_alive(&mut j, "s", 11_000));
        assert!(!on_alive(&mut j, "s", 12_000));
        let mut j3 = job();
        with(&mut j3, "s", ActionKind::Service);
        on_started(&mut j3, "s", 0, None);
        on_stopped(&mut j3, "s", 3_000, None);
        assert!(!j3.actions[0].succeeded);
        on_started(&mut j3, "s", 0, None);
        on_lost(&mut j3, "s", 50_000);
        assert!(!j3.actions[0].succeeded);
        assert_eq!(
            j3.actions[0].last.as_ref().unwrap().state,
            ActionState::Lost
        );
        // Ending something that does not run changes nothing.
        let before = j3.clone();
        on_exited(&mut j3, "s", 60_000, 0);
        assert_eq!(j3, before);
    }

    #[test]
    fn save_offer_lists_succeeded_and_unsaved() {
        let mut j = job();
        for id in ["a", "b", "c"] {
            with(&mut j, id, ActionKind::Task);
            j.actions.last_mut().unwrap().succeeded = id != "c";
        }
        assert_eq!(save_offer(&j, None), ["a", "b"]);
        let same = j.actions[0].def.clone();
        let mut changed = j.actions[1].def.clone();
        changed.run = "other".into();
        assert_eq!(save_offer(&j, Some(&[same, changed])), ["b"]);
    }

    #[test]
    fn profile_actions_start_approved() {
        let a = from_profile(&def());
        assert_eq!(a.source, ActionSource::Profile);
        assert_eq!(a.approved, Some(approval_hash(&def())));
        let mut j = job();
        j.actions = vec![a];
        may_run(&j, "dev", false).unwrap();
    }

    #[test]
    fn names_and_listing() {
        assert_eq!(session_name("feat", "dev"), "xherd-feat-act-dev");
        let mut j = job();
        assert_eq!(ls_text(&j), "No actions registered.");
        with(&mut j, "dev", ActionKind::Service);
        j.actions.push(from_profile(&ActionDef {
            id: "t".into(),
            label: "Tests".into(),
            run: "cargo test".into(),
            cwd: "api".into(),
            ..ActionDef::default()
        }));
        on_started(&mut j, "dev", 0, None);
        on_started(&mut j, "t", 0, None);
        on_exited(&mut j, "t", 1, 3);
        assert_eq!(
            ls_text(&j),
            "dev [service] Dev server | run: npm run dev | cwd: . | source: job | running\n\
             t [task] Tests | run: cargo test | cwd: api | source: profile | exited 3"
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
