//! `GET/PUT /config` (design §7.1): the settings the UI edits, saved to
//! `config.json`. The running daemon keeps the config it started with
//! (actors share it), so a change applies at the next start; `welcomed` is
//! UI-only and needs no restart, like `disabledPlugins` (read live).

use serde::Deserialize;
use serde_json::{Map, Value, json};
use xherd_core::config::{Config, GitIdentity};

use crate::{ActionError, Daemon};

/// Fields the UI may change; anything else is refused.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigPatch {
    pub max_concurrent_agents: Option<usize>,
    pub gate_park_minutes: Option<u64>,
    pub idle_alert_minutes: Option<u64>,
    pub pause_timeout_sec: Option<u64>,
    pub fresh_sessions: Option<bool>,
    pub event_retention_days: Option<u64>,
    /// Empty string = back to the default.
    pub workspace: Option<String>,
    /// Plugin name → its settings object (replaces that plugin's settings).
    pub plugin_settings: Option<Map<String, Value>>,
    /// Names of the plugins that are off (replaces the list).
    pub disabled_plugins: Option<Vec<String>>,
    /// Empty string = none.
    pub git_user_name: Option<String>,
    pub git_user_email: Option<String>,
    /// Replaces the list of identities by remote.
    pub git_identities: Option<Vec<GitIdentity>>,
    pub welcomed: Option<bool>,
    /// Empty string = off.
    pub ntfy_server: Option<String>,
    pub ntfy_topic: Option<String>,
    pub max_session_minutes: Option<u64>,
    pub pr_poll_minutes: Option<u64>,
    pub max_review_loops: Option<u32>,
    pub autonomy: Option<xherd_core::autonomy::Autonomy>,
    pub max_auto_approvals_in_row: Option<u32>,
    pub agent_token_budget: Option<u64>,
    pub daily_token_budget: Option<u64>,
    pub max_diff_lines: Option<usize>,
    /// Replaces the list of sensitive path globs.
    pub sensitive_paths: Option<Vec<String>>,
    /// Tool → default version; an empty version removes the tool's default.
    pub toolchain_defaults: Option<std::collections::BTreeMap<String, String>>,
}

fn bad(message: impl Into<String>) -> ActionError {
    ActionError {
        code: "bad_config",
        message: message.into(),
    }
}

/// `host[:port]` or `host[:port]/owner`: plain characters only, since the
/// scope is written into a git config pattern.
fn valid_scope(s: &str) -> bool {
    let (host, owner) = match s.split_once('/') {
        Some((h, o)) => (h, Some(o)),
        None => (s, None),
    };
    let host_ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
    let owner_ok = owner.is_none_or(|o| {
        !o.is_empty()
            && o.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    });
    host_ok && owner_ok
}

/// Apply a patch with range checks. Pure, so it is tested directly.
pub fn apply(c: &mut Config, p: ConfigPatch) -> Result<(), ActionError> {
    if let Some(n) = p.max_concurrent_agents {
        if !(1..=64).contains(&n) {
            return Err(bad("maxConcurrentAgents must be 1 to 64"));
        }
        c.max_concurrent_agents = n;
    }
    if let Some(n) = p.pause_timeout_sec {
        if !(5..=3600).contains(&n) {
            return Err(bad("pauseTimeoutSec must be 5 to 3600"));
        }
        c.pause_timeout_sec = n;
    }
    if let Some(n) = p.event_retention_days {
        if n == 0 {
            return Err(bad("eventRetentionDays must be at least 1"));
        }
        c.event_retention_days = n;
    }
    if let Some(n) = p.gate_park_minutes {
        c.gate_park_minutes = n;
    }
    if let Some(n) = p.idle_alert_minutes {
        c.idle_alert_minutes = n;
    }
    if let Some(b) = p.fresh_sessions {
        c.fresh_sessions = b;
    }
    if let Some(w) = p.workspace {
        let w = w.trim();
        if !w.is_empty() && !std::path::Path::new(w).is_absolute() {
            return Err(bad("workspace must be an absolute path"));
        }
        c.workspace = (!w.is_empty()).then(|| w.to_string());
    }
    if let Some(ps) = p.plugin_settings {
        for (name, v) in ps {
            if !v.is_object() {
                return Err(bad(format!("settings of {name} must be an object")));
            }
            c.plugin_settings.insert(name, v);
        }
    }
    if let Some(n) = p.pr_poll_minutes {
        c.pr_poll_minutes = n;
    }
    if let Some(n) = p.max_review_loops {
        c.max_review_loops = n;
    }
    if let Some(a) = p.autonomy {
        if !a.selectable() {
            return Err(bad("autonomy never is not available yet (v1.x, R-1209)"));
        }
        c.autonomy = a;
    }
    if let Some(n) = p.max_auto_approvals_in_row {
        c.max_auto_approvals_in_row = n;
    }
    if let Some(n) = p.agent_token_budget {
        c.agent_token_budget = n;
    }
    if let Some(n) = p.daily_token_budget {
        c.daily_token_budget = n;
    }
    if let Some(n) = p.max_session_minutes {
        c.max_session_minutes = n;
    }
    if let Some(n) = p.max_diff_lines {
        if !(1..=1_000_000).contains(&n) {
            return Err(bad("maxDiffLines must be 1 to 1000000"));
        }
        c.max_diff_lines = n;
    }
    if let Some(globs) = p.sensitive_paths {
        let mut out: Vec<String> = Vec::new();
        for g in globs {
            let g = g.trim();
            if g.is_empty()
                || g.len() > 200
                || g.chars().any(char::is_control)
                || g.starts_with('/')
            {
                return Err(bad("sensitivePaths: short relative globs, one per entry"));
            }
            if !out.iter().any(|o| o == g) {
                out.push(g.to_string());
            }
        }
        c.sensitive_paths = out;
    }
    if let Some(defaults) = p.toolchain_defaults {
        let mut out = std::collections::BTreeMap::new();
        for (tool, v) in defaults {
            let v: String = v.chars().filter(|c| !c.is_whitespace()).collect();
            if v.is_empty() {
                continue;
            }
            if !xherd_plugin::toolchain::valid_version(&v) || tool.len() > 40 {
                return Err(bad(format!(
                    "toolchainDefaults: {tool}: not a valid version"
                )));
            }
            out.insert(tool, v);
        }
        c.toolchain_defaults = out;
    }
    if let Some(v) = p.ntfy_server {
        let v = v.trim().trim_end_matches('/');
        if !v.is_empty() && !crate::notifier::valid_server(v) {
            return Err(bad("ntfyServer must be an http(s) URL without spaces"));
        }
        c.ntfy_server = (!v.is_empty()).then(|| v.to_string());
    }
    if let Some(v) = p.ntfy_topic {
        let v = v.trim();
        if !v.is_empty() && !crate::notifier::valid_topic(v) {
            return Err(bad("ntfyTopic: letters, digits, - and _ only (up to 64)"));
        }
        c.ntfy_topic = (!v.is_empty()).then(|| v.to_string());
    }
    if let Some(mut names) = p.disabled_plugins {
        names.sort();
        names.dedup();
        c.disabled_plugins = names;
    }
    for (field, value, what) in [
        (&mut c.git_user_name, p.git_user_name, "gitUserName"),
        (&mut c.git_user_email, p.git_user_email, "gitUserEmail"),
    ] {
        if let Some(v) = value {
            let v = v.trim();
            if v.chars().any(char::is_control) || v.len() > 200 {
                return Err(bad(format!("{what} must be one short line")));
            }
            if what == "gitUserEmail" && !v.is_empty() && !v.contains('@') {
                return Err(bad("gitUserEmail must be an email address"));
            }
            *field = (!v.is_empty()).then(|| v.to_string());
        }
    }
    if let Some(list) = p.git_identities {
        let mut out: Vec<GitIdentity> = Vec::new();
        for i in list {
            let scope = i.scope.trim().trim_end_matches('/').to_string();
            if !valid_scope(&scope) {
                return Err(bad(format!(
                    "{scope:?} is not a host or host/owner (for example github.com/my-company)"
                )));
            }
            let (name, email) = (i.name.trim().to_string(), i.email.trim().to_string());
            if name.is_empty() || !email.contains('@') {
                return Err(bad(format!("{scope}: a name and an email are needed")));
            }
            if [&name, &email]
                .iter()
                .any(|v| v.chars().any(char::is_control) || v.len() > 200)
            {
                return Err(bad(format!(
                    "{scope}: name and email must be one short line"
                )));
            }
            if out.iter().any(|o| o.scope == scope) {
                return Err(bad(format!("{scope} appears twice")));
            }
            out.push(GitIdentity { scope, name, email });
        }
        c.git_identities = out;
    }
    if let Some(b) = p.welcomed {
        c.welcomed = b;
    }
    Ok(())
}

/// The config without the user token.
fn shown(c: &Config) -> Value {
    let mut v = serde_json::to_value(c).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.remove("userToken");
    }
    v
}

impl Daemon {
    fn saved_config(&self) -> Result<Config, ActionError> {
        Config::load(&self.paths.config())
            .map(|(c, _)| c)
            .map_err(|e| ActionError {
                code: "io",
                message: e.to_string(),
            })
    }

    /// `{config, running, restartNeeded}`: what is saved, what runs, and
    /// whether they differ in anything but `welcomed`.
    pub fn config_view(&self) -> Result<Value, ActionError> {
        let saved = self.saved_config()?;
        let mut a = saved.clone();
        a.welcomed = self.config.welcomed;
        a.disabled_plugins = self.config.disabled_plugins.clone();
        a.git_user_name = self.config.git_user_name.clone();
        a.git_user_email = self.config.git_user_email.clone();
        a.git_identities = self.config.git_identities.clone();
        a.ntfy_server = self.config.ntfy_server.clone();
        a.ntfy_topic = self.config.ntfy_topic.clone();
        a.user_token = self.config.user_token.clone();
        Ok(json!({
            "config": shown(&saved),
            "running": shown(&self.config),
            "restartNeeded": a != *self.config,
        }))
    }

    /// Names must exist, and every kind that has plugins keeps one on.
    fn check_disabled(&self, off: &[String]) -> Result<(), ActionError> {
        if let Some(n) = off.iter().find(|n| self.plugins.get(n).is_none()) {
            return Err(bad(format!("no plugin named {n}")));
        }
        let kinds: std::collections::BTreeSet<String> = self
            .plugins
            .all()
            .map(|p| format!("{:?}", p.manifest.kind))
            .collect();
        for k in kinds {
            let any_on = self
                .plugins
                .all()
                .filter(|p| format!("{:?}", p.manifest.kind) == k)
                .any(|p| !off.iter().any(|n| n == p.name()));
            if !any_on {
                return Err(bad(format!(
                    "at least one {} plugin must stay on",
                    k.to_lowercase()
                )));
            }
        }
        Ok(())
    }

    pub fn save_config(&self, patch: ConfigPatch) -> Result<Value, ActionError> {
        let mut c = self.saved_config()?;
        apply(&mut c, patch)?;
        self.check_disabled(&c.disabled_plugins)?;
        c.save(&self.paths.config()).map_err(|e| ActionError {
            code: "io",
            message: e.to_string(),
        })?;
        // The commit identity goes into the managed git config at once.
        if let Err(e) = self.write_gitconfig() {
            tracing::warn!(error = %e.message, "managed git config not written");
        }
        self.config_view()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_ranges_and_fields() {
        let mut c = Config::default();
        let p: ConfigPatch = serde_json::from_value(json!({
            "maxConcurrentAgents": 6, "freshSessions": false, "workspace": "/w",
            "pluginSettings": {"github": {"gh": "/bin/gh"}}, "welcomed": true
        }))
        .unwrap();
        apply(&mut c, p).unwrap();
        assert_eq!(c.max_concurrent_agents, 6);
        assert!(!c.fresh_sessions && c.welcomed);
        assert_eq!(c.workspace.as_deref(), Some("/w"));
        assert_eq!(c.plugin_settings["github"]["gh"], "/bin/gh");

        let p: ConfigPatch = serde_json::from_value(json!({"workspace": ""})).unwrap();
        apply(&mut c, p).unwrap();
        assert_eq!(c.workspace, None);

        for bad in [
            json!({"maxConcurrentAgents": 0}),
            json!({"workspace": "rel/dir"}),
            json!({"pluginSettings": {"x": 1}}),
        ] {
            let p: ConfigPatch = serde_json::from_value(bad).unwrap();
            assert_eq!(apply(&mut c, p).unwrap_err().code, "bad_config");
        }
        assert!(serde_json::from_value::<ConfigPatch>(json!({"port": 1})).is_err());
        assert!(serde_json::from_value::<ConfigPatch>(json!({"userToken": "x"})).is_err());
    }

    #[test]
    fn ntfy_settings_are_validated_and_cleared_by_empty() {
        let mut c = Config::default();
        let p: ConfigPatch = serde_json::from_value(
            json!({"ntfyServer": " https://ntfy.example.com/ ", "ntfyTopic": "my-topic_1"}),
        )
        .unwrap();
        apply(&mut c, p).unwrap();
        assert_eq!(c.ntfy_server.as_deref(), Some("https://ntfy.example.com"));
        assert_eq!(c.ntfy_topic.as_deref(), Some("my-topic_1"));
        for bad in [
            json!({"ntfyServer": "ntfy.sh"}),
            json!({"ntfyTopic": "a/b"}),
        ] {
            let p: ConfigPatch = serde_json::from_value(bad).unwrap();
            assert_eq!(apply(&mut c, p).unwrap_err().code, "bad_config");
        }
        let p: ConfigPatch =
            serde_json::from_value(json!({"ntfyServer": "", "ntfyTopic": ""})).unwrap();
        apply(&mut c, p).unwrap();
        assert_eq!((c.ntfy_server, c.ntfy_topic), (None, None));
    }
}
