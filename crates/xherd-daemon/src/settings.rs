//! `GET/PUT /config` (design §7.1): the settings the UI edits, saved to
//! `config.json`. The running daemon keeps the config it started with
//! (actors share it), so a change applies at the next start; `welcomed` is
//! UI-only and needs no restart.

use serde::Deserialize;
use serde_json::{Map, Value, json};
use xherd_core::config::Config;

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
    pub welcomed: Option<bool>,
}

fn bad(message: impl Into<String>) -> ActionError {
    ActionError {
        code: "bad_config",
        message: message.into(),
    }
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
        a.user_token = self.config.user_token.clone();
        Ok(json!({
            "config": shown(&saved),
            "running": shown(&self.config),
            "restartNeeded": a != *self.config,
        }))
    }

    pub fn save_config(&self, patch: ConfigPatch) -> Result<Value, ActionError> {
        let mut c = self.saved_config()?;
        apply(&mut c, patch)?;
        c.save(&self.paths.config()).map_err(|e| ActionError {
            code: "io",
            message: e.to_string(),
        })?;
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
}
