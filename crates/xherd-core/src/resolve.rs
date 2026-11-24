//! One place that turns settings, repo profile, saved agent and a job's own
//! fields into what a job runs with (issue #141, spec R12, R12a, R13). Pure.
//!
//! Defaults (runtime, model, base, flow): request > agent > profile > settings.
//! Autonomy and permission mode: request > agent > settings, then the
//! profile's value is a ceiling ("at most"). Guardrails are not defaults: the
//! stricter of profile and agent wins.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::agent_def::{AgentDef, JobAgent};
use crate::autonomy::Autonomy;
use crate::new::DEFAULT_FLOW;
use crate::permission::PermissionMode;
use crate::profile::Profile;

/// What the settings say (`config.json`, read live).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    pub runtime: String,
    pub autonomy: Autonomy,
    pub mode: PermissionMode,
    /// `runtimeModels`.
    pub models: BTreeMap<String, String>,
}

/// The job's own fields (form or flags); none = not given.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Requested {
    pub runtime: Option<String>,
    pub model: Option<String>,
    pub autonomy: Option<Autonomy>,
    pub mode: Option<PermissionMode>,
    pub base: Option<String>,
    pub flow: Option<String>,
}

/// A value the profile's ceiling lowered (R12a), for the creation answer and
/// the job page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lowered {
    /// `autonomy` or `permissionMode`.
    pub field: String,
    pub from: String,
    pub to: String,
    /// The profile's name.
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    pub runtime: String,
    pub model: Option<String>,
    pub autonomy: Autonomy,
    pub mode: PermissionMode,
    pub base: Option<String>,
    pub flow: String,
    pub lowered: Vec<Lowered>,
}

/// `chosen`, or `cap` when that is lower; the flag says it was lowered.
pub fn ceiling<T: Ord + Copy>(chosen: T, cap: Option<T>) -> (T, bool) {
    match cap {
        Some(c) if chosen > c => (c, true),
        _ => (chosen, false),
    }
}

fn non_empty(s: &Option<String>) -> Option<&String> {
    s.as_ref().filter(|s| !s.trim().is_empty())
}

pub fn effective(
    d: &Defaults,
    p: Option<&Profile>,
    a: Option<&AgentDef>,
    r: &Requested,
) -> Effective {
    let runtime = non_empty(&r.runtime)
        .or(a.map(|a| &a.runtime))
        .or(p.and_then(|p| non_empty(&p.runtime)))
        .unwrap_or(&d.runtime)
        .clone();
    // The model of the chosen runtime: a model is only meaningful there.
    let model = non_empty(&r.model)
        .or(a.and_then(|a| a.models.get(&runtime)))
        .or(p.and_then(|p| p.models.get(&runtime)))
        .or(d.models.get(&runtime))
        .cloned();
    let base = non_empty(&r.base)
        .or(p.and_then(|p| non_empty(&p.base_branch)))
        .cloned();
    let flow = non_empty(&r.flow)
        .cloned()
        .unwrap_or_else(|| DEFAULT_FLOW.into());

    let want_autonomy = r
        .autonomy
        .or(a.and_then(|a| a.autonomy))
        .unwrap_or(d.autonomy);
    let want_mode = r
        .mode
        .or(a.and_then(|a| a.permission_mode))
        .unwrap_or(d.mode);
    let (autonomy, a_low) = ceiling(want_autonomy, p.and_then(|p| p.autonomy));
    let (mode, m_low) = ceiling(want_mode, p.and_then(|p| p.permission_mode));
    let mut lowered = Vec::new();
    if let Some(p) = p {
        if a_low {
            lowered.push(Lowered {
                field: "autonomy".into(),
                from: want_autonomy.to_string(),
                to: autonomy.to_string(),
                profile: p.name.clone(),
            });
        }
        if m_low {
            lowered.push(Lowered {
                field: "permissionMode".into(),
                from: want_mode.to_string(),
                to: mode.to_string(),
                profile: p.name.clone(),
            });
        }
    }
    Effective {
        runtime,
        model,
        autonomy,
        mode,
        base,
        flow,
        lowered,
    }
}

/// The three limits the actor checks (R13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guardrails {
    pub sensitive_paths: Vec<String>,
    pub max_diff_lines: usize,
    /// 0 = off.
    pub job_token_budget: u64,
}

/// The profile's values, else the global ones (a profile's empty
/// `sensitivePaths` still switches the global list off), then the stricter of
/// that and the agent's: paths are joined, the smaller diff limit wins, and
/// the smallest non-zero budget (0 = off loses to any limit). Paths and the
/// diff limit are the current stage's agent's (`a`); the token budget is the
/// job's, so every agent of the job (`all`) counts.
pub fn guardrails(
    global: &Guardrails,
    p: Option<&Profile>,
    a: Option<&JobAgent>,
    all: &BTreeMap<String, JobAgent>,
) -> Guardrails {
    let mut g = Guardrails {
        sensitive_paths: p
            .and_then(|p| p.sensitive_paths.clone())
            .unwrap_or_else(|| global.sensitive_paths.clone()),
        max_diff_lines: p
            .and_then(|p| p.max_diff_lines)
            .unwrap_or(global.max_diff_lines),
        job_token_budget: p
            .and_then(|p| p.job_token_budget)
            .unwrap_or(global.job_token_budget),
    };
    if let Some(a) = a {
        for path in &a.sensitive_paths {
            if !g.sensitive_paths.contains(path) {
                g.sensitive_paths.push(path.clone());
            }
        }
        if let Some(n) = a.max_diff_lines {
            g.max_diff_lines = g.max_diff_lines.min(n);
        }
    }
    for b in all
        .values()
        .filter_map(|a| a.job_token_budget)
        .filter(|b| *b > 0)
    {
        g.job_token_budget = match g.job_token_budget {
            0 => b,
            have => have.min(b),
        };
    }
    g
}

/// The runtime an agent runs on in a new job (R36): its own, else its
/// fallback when that one cannot run now. `None` = neither can. The bool says
/// the fallback was used. Chosen once, at creation: nothing switches later.
pub fn usable_runtime(
    runtime: &str,
    fallback: Option<&str>,
    is_usable: impl Fn(&str) -> bool,
) -> Option<(String, bool)> {
    if is_usable(runtime) {
        return Some((runtime.to_string(), false));
    }
    fallback
        .filter(|f| !f.is_empty() && is_usable(f))
        .map(|f| (f.to_string(), true))
}

/// One stage of a workflow given to a saved agent (D26, R35). `human` makes
/// the stage's gate wait for the user whatever the autonomy level says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Assignment {
    pub stage: String,
    pub agent_id: String,
    #[serde(default)]
    pub human: bool,
}

/// The agent that runs `stage` (R37): the request's override for that stage,
/// else its assignment, else the job's default agent (`--agent A`), else the
/// agent of the stage before. `None` = no agent (a plain job).
pub fn stage_agent(
    assignments: &[Assignment],
    overrides: &BTreeMap<String, String>,
    default: Option<&str>,
    previous: Option<&str>,
    stage: &str,
) -> Option<String> {
    overrides
        .get(stage)
        .map(String::as_str)
        .or_else(|| {
            assignments
                .iter()
                .find(|a| a.stage == stage)
                .map(|a| a.agent_id.as_str())
        })
        .or(default)
        .or(previous)
        .map(str::to_string)
}

/// `stage_agent` for every stage of a flow in order, each stage's answer
/// being the next one's `previous`.
pub fn stage_agents(
    stages: &[&str],
    assignments: &[Assignment],
    overrides: &BTreeMap<String, String>,
    default: Option<&str>,
) -> Vec<Option<String>> {
    let mut out: Vec<Option<String>> = Vec::with_capacity(stages.len());
    for stage in stages {
        let previous = out.last().and_then(|p| p.as_deref());
        out.push(stage_agent(
            assignments,
            overrides,
            default,
            previous,
            stage,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Defaults {
        Defaults {
            runtime: "claude-code".into(),
            autonomy: Autonomy::Ask,
            mode: PermissionMode::Edits,
            models: BTreeMap::from([("claude-code".into(), "settings-model".into())]),
        }
    }

    fn profile() -> Profile {
        Profile {
            name: "acme".into(),
            ..Profile::default()
        }
    }

    fn agent() -> AgentDef {
        AgentDef {
            id: "a".into(),
            name: "rev".into(),
            description: String::new(),
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

    #[test]
    fn the_enums_are_ordered_loosest_last() {
        assert!(Autonomy::Ask < Autonomy::Important && Autonomy::Important < Autonomy::Never);
        assert!(
            PermissionMode::Ask < PermissionMode::Edits
                && PermissionMode::Edits < PermissionMode::Auto
                && PermissionMode::Auto < PermissionMode::All
        );
        // `ALL` is declared in the same order.
        let mut sorted = PermissionMode::ALL;
        sorted.sort();
        assert_eq!(sorted, PermissionMode::ALL);
    }

    #[test]
    fn ceiling_lowers_only_a_looser_value() {
        assert_eq!(ceiling(3, Some(2)), (2, true));
        assert_eq!(ceiling(2, Some(2)), (2, false));
        assert_eq!(ceiling(1, Some(2)), (1, false));
        assert_eq!(ceiling(3, None), (3, false));
    }

    #[test]
    fn settings_alone() {
        let e = effective(&defaults(), None, None, &Requested::default());
        assert_eq!(e.runtime, "claude-code");
        assert_eq!(e.model.as_deref(), Some("settings-model"));
        assert_eq!((e.autonomy, e.mode), (Autonomy::Ask, PermissionMode::Edits));
        assert_eq!((e.base, e.flow.as_str()), (None, "feature"));
        assert!(e.lowered.is_empty());
    }

    #[test]
    fn defaults_go_request_agent_profile_settings() {
        let mut p = profile();
        p.runtime = Some("opencode".into());
        p.base_branch = Some("p-base".into());
        p.models.insert("opencode".into(), "p-model".into());
        let d = defaults();
        // profile beats settings
        let e = effective(&d, Some(&p), None, &Requested::default());
        assert_eq!(e.runtime, "opencode");
        assert_eq!(e.model.as_deref(), Some("p-model"));
        assert_eq!(e.base.as_deref(), Some("p-base"));
        // agent beats profile (the base and the flow are not the agent's)
        let mut a = agent();
        a.runtime = "codex".into();
        a.models.insert("codex".into(), "a-model".into());
        let e = effective(&d, Some(&p), Some(&a), &Requested::default());
        assert_eq!(e.runtime, "codex");
        assert_eq!(e.model.as_deref(), Some("a-model"));
        assert_eq!(e.base.as_deref(), Some("p-base"));
        assert_eq!(e.flow, "feature");
        // the request beats the agent
        let r = Requested {
            runtime: Some("opencode".into()),
            model: Some("r-model".into()),
            base: Some("r-base".into()),
            flow: Some("none".into()),
            ..Requested::default()
        };
        let e = effective(&d, Some(&p), Some(&a), &r);
        assert_eq!(e.runtime, "opencode");
        assert_eq!(e.model.as_deref(), Some("r-model"));
        assert_eq!(e.base.as_deref(), Some("r-base"));
        assert_eq!(e.flow, "none");
    }

    #[test]
    fn the_model_belongs_to_the_chosen_runtime() {
        let d = defaults();
        let mut a = agent();
        a.models.insert("claude-code".into(), "a-claude".into());
        // the request picks another runtime: the agent's claude model is not used,
        // and neither is the settings' (it is for claude-code too)
        let r = Requested {
            runtime: Some("opencode".into()),
            ..Requested::default()
        };
        assert_eq!(effective(&d, None, Some(&a), &r).model, None);
        // empty strings count as not given
        let r = Requested {
            runtime: Some(" ".into()),
            model: Some(String::new()),
            ..Requested::default()
        };
        let e = effective(&d, None, Some(&a), &r);
        assert_eq!(e.runtime, "claude-code");
        assert_eq!(e.model.as_deref(), Some("a-claude"));
    }

    #[test]
    fn autonomy_and_mode_go_request_agent_settings_then_the_ceiling() {
        let d = defaults();
        let mut a = agent();
        a.autonomy = Some(Autonomy::Important);
        a.permission_mode = Some(PermissionMode::Auto);
        let e = effective(&d, None, Some(&a), &Requested::default());
        assert_eq!(
            (e.autonomy, e.mode),
            (Autonomy::Important, PermissionMode::Auto)
        );
        let r = Requested {
            autonomy: Some(Autonomy::Ask),
            mode: Some(PermissionMode::Ask),
            ..Requested::default()
        };
        let e = effective(&d, None, Some(&a), &r);
        assert_eq!((e.autonomy, e.mode), (Autonomy::Ask, PermissionMode::Ask));
    }

    #[test]
    fn a_profile_is_a_ceiling_not_a_default() {
        let d = defaults();
        let mut p = profile();
        p.autonomy = Some(Autonomy::Important);
        p.permission_mode = Some(PermissionMode::Auto);
        // it does not supply a value: settings (ask, edits) stay
        let e = effective(&d, Some(&p), None, &Requested::default());
        assert_eq!((e.autonomy, e.mode), (Autonomy::Ask, PermissionMode::Edits));
        assert!(e.lowered.is_empty());
        // a looser request is lowered, and the answer says so
        let r = Requested {
            autonomy: Some(Autonomy::Never),
            mode: Some(PermissionMode::All),
            ..Requested::default()
        };
        let e = effective(&d, Some(&p), None, &r);
        assert_eq!(
            (e.autonomy, e.mode),
            (Autonomy::Important, PermissionMode::Auto)
        );
        assert_eq!(
            e.lowered,
            vec![
                Lowered {
                    field: "autonomy".into(),
                    from: "never".into(),
                    to: "important".into(),
                    profile: "acme".into()
                },
                Lowered {
                    field: "permissionMode".into(),
                    from: "all".into(),
                    to: "auto".into(),
                    profile: "acme".into()
                },
            ]
        );
        // the agent is capped too, and a stricter request passes untouched
        let mut a = agent();
        a.permission_mode = Some(PermissionMode::All);
        let e = effective(&d, Some(&p), Some(&a), &Requested::default());
        assert_eq!(e.mode, PermissionMode::Auto);
        assert_eq!(e.lowered.len(), 1);
        let r = Requested {
            mode: Some(PermissionMode::Ask),
            ..Requested::default()
        };
        let e = effective(&d, Some(&p), Some(&a), &r);
        assert_eq!(e.mode, PermissionMode::Ask);
        assert!(e.lowered.is_empty());
    }

    fn global() -> Guardrails {
        Guardrails {
            sensitive_paths: vec!["*.pem".into()],
            max_diff_lines: 1000,
            job_token_budget: 0,
        }
    }

    fn snap() -> JobAgent {
        let mut a = agent();
        a.sensitive_paths = vec!["secrets/**".into(), "*.pem".into()];
        a.max_diff_lines = Some(300);
        a.job_token_budget = Some(5000);
        a.snapshot()
    }

    /// `guardrails` for a job with just this one agent.
    fn one(g: &Guardrails, p: Option<&Profile>, a: Option<&JobAgent>) -> Guardrails {
        let all = a.map(|a| (a.id.clone(), a.clone())).into_iter().collect();
        guardrails(g, p, a, &all)
    }

    #[test]
    fn the_job_budget_is_the_lowest_of_all_its_agents_the_rest_the_current_ones() {
        let (mut a, mut b) = (agent(), agent());
        a.id = "a".into();
        a.max_diff_lines = Some(300);
        a.job_token_budget = Some(5000);
        b.id = "b".into();
        b.sensitive_paths = vec!["x/**".into()];
        b.max_diff_lines = Some(900);
        b.job_token_budget = Some(2000);
        let (a, b) = (a.snapshot(), b.snapshot());
        let all = BTreeMap::from([("a".to_string(), a.clone()), ("b".to_string(), b.clone())]);
        let g = guardrails(&global(), None, Some(&a), &all);
        assert_eq!(g.max_diff_lines, 300);
        assert!(g.sensitive_paths.iter().all(|p| p != "x/**"));
        assert_eq!(g.job_token_budget, 2000);
        let g = guardrails(&global(), None, Some(&b), &all);
        assert_eq!(g.max_diff_lines, 900);
        assert!(g.sensitive_paths.contains(&"x/**".to_string()));
    }

    #[test]
    fn guardrails_without_profile_or_agent_are_the_global_ones() {
        assert_eq!(one(&global(), None, None), global());
    }

    #[test]
    fn the_profile_replaces_the_global_values() {
        let mut p = profile();
        p.sensitive_paths = Some(vec![]);
        p.max_diff_lines = Some(50);
        p.job_token_budget = Some(10);
        let g = one(&global(), Some(&p), None);
        assert_eq!(
            g,
            Guardrails {
                sensitive_paths: vec![],
                max_diff_lines: 50,
                job_token_budget: 10
            }
        );
    }

    #[test]
    fn the_stricter_value_wins_between_profile_and_agent() {
        // agent adds paths (no duplicates), lowers the limit, sets a budget
        let g = one(&global(), None, Some(&snap()));
        assert_eq!(g.sensitive_paths, vec!["*.pem", "secrets/**"]);
        assert_eq!(g.max_diff_lines, 300);
        assert_eq!(g.job_token_budget, 5000);
        // a stricter profile is kept; the agent cannot loosen it
        let mut p = profile();
        p.max_diff_lines = Some(100);
        p.job_token_budget = Some(2000);
        p.sensitive_paths = Some(vec!["a".into()]);
        let g = one(&global(), Some(&p), Some(&snap()));
        assert_eq!(g.max_diff_lines, 100);
        assert_eq!(g.job_token_budget, 2000);
        assert_eq!(g.sensitive_paths, vec!["a", "secrets/**", "*.pem"]);
        // an agent with no limits changes nothing; budget 0 is no limit
        let mut a = agent();
        a.job_token_budget = Some(0);
        let g = one(&global(), Some(&p), Some(&a.snapshot()));
        assert_eq!(g.job_token_budget, 2000);
        assert_eq!(g.max_diff_lines, 100);
        let g = one(&global(), None, Some(&a.snapshot()));
        assert_eq!(g.job_token_budget, 0);
    }

    #[test]
    fn a_runtime_that_cannot_run_gives_way_to_the_fallback_once() {
        let on = |r: &str| r == "opencode";
        assert_eq!(
            usable_runtime("opencode", Some("codex"), on),
            Some(("opencode".into(), false))
        );
        assert_eq!(
            usable_runtime("claude-code", Some("opencode"), on),
            Some(("opencode".into(), true))
        );
        assert_eq!(usable_runtime("claude-code", Some("codex"), on), None);
        assert_eq!(usable_runtime("claude-code", None, on), None);
        assert_eq!(usable_runtime("claude-code", Some(""), on), None);
    }

    fn assign(stage: &str, agent: &str) -> Assignment {
        Assignment {
            stage: stage.into(),
            agent_id: agent.into(),
            human: false,
        }
    }

    #[test]
    fn a_stage_takes_override_then_assignment_then_default_then_previous() {
        let asg = [assign("design", "arch"), assign("review", "rev")];
        let ov = BTreeMap::from([("design".to_string(), "mine".to_string())]);
        let none = BTreeMap::new();
        let at = |ov: &BTreeMap<String, String>, d, p, s| stage_agent(&asg, ov, d, p, s);
        // the override beats the assignment, the assignment beats the default
        assert_eq!(
            at(&ov, Some("dflt"), None, "design").as_deref(),
            Some("mine")
        );
        assert_eq!(
            at(&none, Some("dflt"), None, "design").as_deref(),
            Some("arch")
        );
        // no assignment: the default, then the previous stage's agent
        assert_eq!(
            at(&none, Some("dflt"), Some("arch"), "tasks").as_deref(),
            Some("dflt")
        );
        assert_eq!(
            at(&none, None, Some("arch"), "tasks").as_deref(),
            Some("arch")
        );
        assert_eq!(at(&none, None, None, "tasks"), None);
    }

    #[test]
    fn stages_in_order_carry_the_previous_agent_forward() {
        let asg = [assign("design", "arch"), assign("review", "rev")];
        let stages = ["spec", "design", "tasks", "implement", "review", "ship"];
        let got = stage_agents(&stages, &asg, &BTreeMap::new(), None);
        let want: Vec<Option<&str>> = vec![
            None,
            Some("arch"),
            Some("arch"),
            Some("arch"),
            Some("rev"),
            Some("rev"),
        ];
        assert_eq!(got.iter().map(|a| a.as_deref()).collect::<Vec<_>>(), want);
        // with a default, only the assigned stages differ from it
        let got = stage_agents(&stages, &asg, &BTreeMap::new(), Some("dflt"));
        let names: Vec<&str> = got.iter().map(|a| a.as_deref().unwrap()).collect();
        assert_eq!(names, ["dflt", "arch", "dflt", "dflt", "rev", "dflt"]);
    }
}
