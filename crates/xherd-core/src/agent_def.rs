//! Saved agents (issue #141, spec R10…R21): a named, versioned setup that
//! jobs start from. Pure: the record, its checks, the derived status and the
//! default job name. Storage is in `xherd-store`, plugin and pack checks in
//! the daemon (`Daemon::check_agent`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::autonomy::Autonomy;
use crate::job::Millis;
use crate::permission::PermissionMode;
use crate::xap::{Refusal, refuse};
use crate::{Job, new};

pub const MAX_DESCRIPTION: usize = 200;
pub const MAX_INSTRUCTIONS: usize = crate::profile::MAX_INSTRUCTIONS;
/// What an agent may remember (R28), in characters.
pub const MAX_MEMORY: usize = 4000;
pub const MAX_PATHS: usize = 50;
pub const MAX_LIVE_JOBS: usize = 100;

fn one() -> usize {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AgentDef {
    /// Assigned on creation; ignored on import.
    #[serde(default)]
    pub id: String,
    /// A slug like a job's name, unique among agents, fixed after creation.
    pub name: String,
    /// One line.
    #[serde(default)]
    pub description: String,
    /// 1 on creation, bumped on each edit (R11).
    #[serde(default = "one_u32")]
    pub version: u32,
    pub runtime: String,
    /// Runtime plugin → model.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, String>,
    /// Used once, at job creation, when `runtime` is switched off (R36).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_runtime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autonomy: Option<Autonomy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    /// Added to the job's `instructions.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_token_budget: Option<u64>,
    /// Added to the repository's (R13).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sensitive_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_diff_lines: Option<usize>,
    /// Names in the skills library (phase 3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    /// Live jobs at once (R16); at least 1.
    #[serde(default = "one")]
    pub max_live_jobs: usize,
    #[serde(default)]
    pub created_at: Millis,
    #[serde(default)]
    pub updated_at: Millis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<Millis>,
    /// The flow plugin that installed this agent; its name is then
    /// `<source>-<label>`. `None` for agents made by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// The short name of an agent to show in running and past work: the name
/// without its `<source>-` prefix.
pub fn label<'a>(name: &'a str, source: Option<&str>) -> &'a str {
    source
        .and_then(|s| name.strip_prefix(s))
        .and_then(|n| n.strip_prefix('-'))
        .filter(|n| !n.is_empty())
        .unwrap_or(name)
}

fn one_u32() -> u32 {
    1
}

/// What a job keeps of its agent (R12): everything the actor needs later, so
/// editing or deleting the agent never changes a running job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobAgent {
    pub id: String,
    pub name: String,
    pub version: u32,
    /// What the job runs this agent on (R36, R40): the agent's runtime, or its
    /// fallback when that one was off at creation. Empty in snapshots from
    /// before stage agents.
    #[serde(default)]
    pub runtime: String,
    #[serde(default)]
    pub fell_back: bool,
    /// The values the job resolved for this agent at creation (request >
    /// agent > settings, under the profile's ceilings); `launch` is the
    /// runtime's argv with them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autonomy: Option<Autonomy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub launch: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sensitive_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_diff_lines: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_token_budget: Option<u64>,
    /// What became of each skill the agent lists (R26, R27): filled when the
    /// job is created, never by `snapshot`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<SkillCopy>,
    /// The agent's memory when the job was created (R28), never changed after.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    /// The flow plugin the agent came from (see `AgentDef::source`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// The agent a workflow stage runs with in one job, and whether the stage's
/// gate always waits for the user (D26, R35).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageAgent {
    pub agent: String,
    #[serde(default)]
    pub human: bool,
}

/// One skill of an agent at job creation: copied into the worktree for the
/// job's runtime (`installed`), or skipped with the reason (R26, R27).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCopy {
    pub name: String,
    /// Hash of the library copy that was installed, or looked at.
    pub sha256: String,
    pub installed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

impl AgentDef {
    pub fn snapshot(&self) -> JobAgent {
        JobAgent {
            id: self.id.clone(),
            name: self.name.clone(),
            version: self.version,
            runtime: self.runtime.clone(),
            fell_back: false,
            autonomy: None,
            permission_mode: None,
            model: None,
            launch: vec![],
            instructions: self.instructions.clone(),
            sensitive_paths: self.sensitive_paths.clone(),
            max_diff_lines: self.max_diff_lines,
            job_token_budget: self.job_token_budget,
            skills: vec![],
            memory: None,
            source: self.source.clone(),
        }
    }
}

/// An agent's state, derived from its jobs and never stored (R17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentStatus {
    Archived,
    NeedsYou,
    Working,
    Idle,
}

fn bad(message: impl Into<String>) -> Refusal {
    refuse("bad_agent", message)
}

/// The checks that need no plugin or pack (R10). Models are cleaned in place.
pub fn check(def: &mut AgentDef) -> Result<(), Refusal> {
    def.name = def.name.trim().to_string();
    new::check_name(&def.name)?;
    def.description = def.description.trim().to_string();
    if def.description.chars().count() > MAX_DESCRIPTION || def.description.contains('\n') {
        return Err(bad(format!(
            "description: one line, up to {MAX_DESCRIPTION} characters"
        )));
    }
    def.runtime = def.runtime.trim().to_string();
    if def.runtime.is_empty() {
        return Err(bad("a runtime is needed"));
    }
    def.models = crate::model::clean_map(std::mem::take(&mut def.models)).map_err(bad)?;
    if let Some(f) = def.fallback_runtime.as_deref() {
        let f = f.trim();
        if f.is_empty() {
            def.fallback_runtime = None;
        } else if f == def.runtime
            || !f
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(refuse(
                "bad_fallback",
                "fallbackRuntime: another runtime plugin's name",
            ));
        } else {
            def.fallback_runtime = Some(f.to_string());
        }
    }
    if let Some(a) = def.autonomy
        && !a.selectable()
    {
        return Err(refuse(
            "autonomy_unavailable",
            format!("autonomy {a} is not available yet"),
        ));
    }
    if let Some(i) = def.instructions.as_deref() {
        if i.trim().is_empty() {
            def.instructions = None;
        } else if i.chars().count() > MAX_INSTRUCTIONS || i.contains('\0') {
            return Err(bad(format!(
                "instructions: up to {MAX_INSTRUCTIONS} characters"
            )));
        }
    }
    if def.sensitive_paths.len() > MAX_PATHS
        || def
            .sensitive_paths
            .iter()
            .any(|p| p.trim().is_empty() || p.contains(['\n', '\0']))
    {
        return Err(bad(format!(
            "sensitivePaths: up to {MAX_PATHS} plain patterns"
        )));
    }
    if def.max_diff_lines == Some(0) {
        return Err(bad("maxDiffLines: at least 1"));
    }
    if def.max_live_jobs == 0 || def.max_live_jobs > MAX_LIVE_JOBS {
        return Err(bad(format!("maxLiveJobs: 1 to {MAX_LIVE_JOBS}")));
    }
    Ok(())
}

/// `archived`, else `needs you` when a job whose current stage this agent
/// runs does, else `working` when one is live, else `idle` (R17). Finished jobs
/// do not count, and neither does a job that only used the agent for another
/// stage: the agent of the stage that waits is the one that needs you.
pub fn derived_status(def: &AgentDef, jobs: &[&Job]) -> AgentStatus {
    use crate::Status::{Cancelled, Done};
    let mine = |j: &&&Job| j.current_agent.as_deref() == Some(def.id.as_str());
    if def.archived_at.is_some() {
        AgentStatus::Archived
    } else if jobs
        .iter()
        .filter(mine)
        .any(|j| !matches!(j.status, Done | Cancelled) && j.needs_you())
    {
        AgentStatus::NeedsYou
    } else if jobs.iter().filter(mine).any(|j| j.status.is_live()) {
        AgentStatus::Working
    } else {
        AgentStatus::Idle
    }
}

/// The name of a job started from `agent` with none given (R21):
/// `<agent>-<ticket key lower-cased>`, else `<agent>-<n>` with the first free
/// `n` from 1. `taken` says whether a job name exists. A name over the slug
/// rule (a long agent name, a key with odd characters) falls back to the
/// numbered form; the numbered form is cut to fit.
pub fn default_job_name(
    agent: &str,
    ticket_key: Option<&str>,
    taken: &dyn Fn(&str) -> bool,
) -> String {
    let fits = |n: &str| new::check_name(n).is_ok();
    if let Some(k) = ticket_key {
        let key = k
            .trim_start_matches('#')
            .to_ascii_lowercase()
            .replace('_', "-");
        let name = format!("{agent}-{key}");
        if !key.is_empty() && fits(&name) && !taken(&name) {
            return name;
        }
    }
    for n in 1u32.. {
        let suffix = format!("-{n}");
        // 41 characters at most (`NAME`); keep the agent's start.
        let room = 41usize.saturating_sub(suffix.len());
        let head: String = agent.chars().take(room).collect();
        let name = format!("{head}{suffix}");
        if !taken(&name) {
            return name;
        }
    }
    unreachable!("u32 names exhausted")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Status;

    fn def() -> AgentDef {
        AgentDef {
            id: "a1".into(),
            name: "reviewer".into(),
            description: "Reviews branches".into(),
            version: 1,
            runtime: "claude-code".into(),
            models: BTreeMap::new(),
            fallback_runtime: None,
            autonomy: None,
            permission_mode: None,
            instructions: None,
            job_token_budget: None,
            sensitive_paths: vec![],
            max_diff_lines: None,
            skills: vec![],
            max_live_jobs: 1,
            created_at: 0,
            updated_at: 0,
            archived_at: None,
            source: None,
        }
    }

    fn job(status: Status, gate: bool) -> Job {
        let mut j = Job::new("j", 0);
        j.status = status;
        j.current_agent = Some("a1".into());
        if gate {
            j.gate = Some(crate::domain::Gate::Question { text: "q".into() });
        }
        j
    }

    #[test]
    fn a_plain_agent_passes_and_is_trimmed() {
        let mut d = def();
        d.name = " reviewer ".into();
        d.instructions = Some("  ".into());
        d.models.insert("claude-code".into(), " opus ".into());
        check(&mut d).unwrap();
        assert_eq!(d.name, "reviewer");
        assert_eq!(d.instructions, None);
        assert_eq!(d.models["claude-code"], "opus");
    }

    #[test]
    fn refusals() {
        type Edit = Box<dyn Fn(&mut AgentDef)>;
        let cases: Vec<(&str, Edit)> = vec![
            ("bad_name", Box::new(|d| d.name = "Bad Name".into())),
            ("bad_name", Box::new(|d| d.name = String::new())),
            ("bad_agent", Box::new(|d| d.description = "a\nb".into())),
            ("bad_agent", Box::new(|d| d.description = "x".repeat(201))),
            ("bad_agent", Box::new(|d| d.runtime = " ".into())),
            (
                "bad_agent",
                Box::new(|d| {
                    d.models.insert("claude-code".into(), "-x".into());
                }),
            ),
            (
                "bad_fallback",
                Box::new(|d| d.fallback_runtime = Some("claude-code".into())),
            ),
            (
                "bad_fallback",
                Box::new(|d| d.fallback_runtime = Some("Bad Runtime".into())),
            ),
            (
                "autonomy_unavailable",
                Box::new(|d| d.autonomy = Some(Autonomy::Never)),
            ),
            (
                "bad_agent",
                Box::new(|d| d.instructions = Some("x".repeat(MAX_INSTRUCTIONS + 1))),
            ),
            (
                "bad_agent",
                Box::new(|d| d.sensitive_paths = vec![" ".into()]),
            ),
            ("bad_agent", Box::new(|d| d.max_diff_lines = Some(0))),
            ("bad_agent", Box::new(|d| d.max_live_jobs = 0)),
            ("bad_agent", Box::new(|d| d.max_live_jobs = 101)),
        ];
        for (code, edit) in cases {
            let mut d = def();
            edit(&mut d);
            assert_eq!(check(&mut d).unwrap_err().code, code, "{d:?}");
        }
    }

    #[test]
    fn the_fallback_runtime_is_trimmed_and_may_be_empty() {
        let mut d = def();
        d.fallback_runtime = Some(" opencode ".into());
        check(&mut d).unwrap();
        assert_eq!(d.fallback_runtime.as_deref(), Some("opencode"));
        d.fallback_runtime = Some("  ".into());
        check(&mut d).unwrap();
        assert_eq!(d.fallback_runtime, None);
    }

    #[test]
    fn import_refuses_unknown_fields_and_fills_defaults() {
        let d: AgentDef = serde_json::from_str(r#"{"name":"a","runtime":"claude-code"}"#).unwrap();
        assert_eq!((d.version, d.max_live_jobs), (1, 1));
        assert!(
            serde_json::from_str::<AgentDef>(r#"{"name":"a","runtime":"r","surprise":1}"#).is_err()
        );
        // the fields the agent lost with stage agents are no longer known
        for key in ["workflow", "repo", "baseBranch", "stageOverrides"] {
            let json = format!(r#"{{"name":"a","runtime":"r","{key}":null}}"#);
            assert!(serde_json::from_str::<AgentDef>(&json).is_err(), "{key}");
        }
    }

    #[test]
    fn status_is_derived() {
        let d = def();
        assert_eq!(derived_status(&d, &[]), AgentStatus::Idle);
        let running = job(Status::Running, false);
        let waiting = job(Status::WaitingInput, false);
        let gated = job(Status::Idle, true);
        let paused = job(Status::Paused, false);
        let done = job(Status::Done, true);
        assert_eq!(derived_status(&d, &[&paused, &done]), AgentStatus::Idle);
        assert_eq!(
            derived_status(&d, &[&paused, &running]),
            AgentStatus::Working
        );
        assert_eq!(
            derived_status(&d, &[&running, &waiting]),
            AgentStatus::NeedsYou
        );
        assert_eq!(derived_status(&d, &[&gated]), AgentStatus::NeedsYou);
        let mut archived = def();
        archived.archived_at = Some(5);
        assert_eq!(
            derived_status(&archived, &[&waiting]),
            AgentStatus::Archived
        );
    }

    #[test]
    fn only_the_agent_of_the_current_stage_needs_you() {
        let d = def();
        // the job used this agent before, but another one runs its stage now
        let mut gated = job(Status::Idle, true);
        gated.current_agent = Some("other".into());
        assert_eq!(derived_status(&d, &[&gated]), AgentStatus::Idle);
        let mut running = job(Status::Running, false);
        running.current_agent = None;
        assert_eq!(derived_status(&d, &[&running]), AgentStatus::Idle);
    }

    #[test]
    fn default_names() {
        let free = |_: &str| false;
        assert_eq!(
            default_job_name("rev", Some("PAY-412"), &free),
            "rev-pay-412"
        );
        assert_eq!(default_job_name("rev", Some("#12"), &free), "rev-12");
        assert_eq!(default_job_name("rev", None, &free), "rev-1");
        let taken = |n: &str| n == "rev-1" || n == "rev-2" || n == "rev-pay-1";
        assert_eq!(default_job_name("rev", None, &taken), "rev-3");
        assert_eq!(default_job_name("rev", Some("PAY-1"), &taken), "rev-3");
        // too long for the slug rule: numbered, cut to fit
        let long = "a".repeat(41);
        let n = default_job_name(&long, Some("PAY-1"), &free);
        assert_eq!(n.len(), 41);
        assert!(n.ends_with("-1"));
        assert!(new::check_name(&n).is_ok());
    }

    #[test]
    fn the_snapshot_keeps_what_the_actor_needs() {
        let mut d = def();
        d.instructions = Some("be brief".into());
        d.sensitive_paths = vec!["*.pem".into()];
        d.max_diff_lines = Some(200);
        d.job_token_budget = Some(5);
        let s = d.snapshot();
        assert_eq!((s.name.as_str(), s.version), ("reviewer", 1));
        assert_eq!(s.max_diff_lines, Some(200));
        assert_eq!((s.runtime.as_str(), s.fell_back), ("claude-code", false));
        let back: JobAgent = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn source_is_kept_by_the_snapshot_and_skipped_when_none() {
        let mut d = def();
        assert!(!serde_json::to_string(&d).unwrap().contains("source"));
        assert!(
            !serde_json::to_string(&d.snapshot())
                .unwrap()
                .contains("source")
        );
        d.source = Some("acme-sdd".into());
        let s = d.snapshot();
        assert_eq!(s.source.as_deref(), Some("acme-sdd"));
        let back: JobAgent = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        let d2: AgentDef = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(d2, d);
    }

    #[test]
    fn label_strips_the_source_prefix() {
        assert_eq!(label("acme-sdd-architect", Some("acme-sdd")), "architect");
        assert_eq!(label("architect", None), "architect");
        // a name that does not carry the prefix, or is only the prefix, stays whole
        assert_eq!(label("architect", Some("acme-sdd")), "architect");
        assert_eq!(label("acme-sdd-", Some("acme-sdd")), "acme-sdd-");
        assert_eq!(label("acme-sddx", Some("acme-sdd")), "acme-sddx");
    }

    #[test]
    fn skill_copies_round_trip_and_old_snapshots_have_none() {
        let mut s = def().snapshot();
        assert!(s.skills.is_empty());
        assert!(!serde_json::to_string(&s).unwrap().contains("skills"));
        s.skills = vec![
            SkillCopy {
                name: "a".into(),
                sha256: "x".into(),
                installed: true,
                skipped: None,
            },
            SkillCopy {
                name: "b".into(),
                sha256: "y".into(),
                installed: false,
                skipped: Some("the repository has its own".into()),
            },
        ];
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["skills"][1]["skipped"], "the repository has its own");
        assert!(json["skills"][0].get("skipped").is_none());
        let back: JobAgent = serde_json::from_value(json).unwrap();
        assert_eq!(back, s);
    }
}
