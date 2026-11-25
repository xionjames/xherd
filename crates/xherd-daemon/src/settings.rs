//! `GET/PUT /config` (design §7.1): the settings the UI edits, saved to
//! `config.json`. The running daemon keeps the config it started with
//! (actors share it), so a change applies at the next start; `welcomed` is
//! UI-only and needs no restart, like `disabledPlugins` (read live).

use serde::Deserialize;
use serde_json::{Map, Value, json};
use xherd_core::config::{Config, GitIdentity};

use crate::actor::ActorSettings;
use crate::{ActionError, Daemon};

/// Fields the UI may change; anything else is refused.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigPatch {
    #[serde(alias = "maxConcurrentAgents")]
    pub max_concurrent_jobs: Option<usize>,
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
    pub muted_notifications: Option<Vec<String>>,
    pub desktop_notifications: Option<bool>,
    pub suggest_agents: Option<bool>,
    pub max_session_minutes: Option<u64>,
    pub pr_poll_minutes: Option<u64>,
    pub max_review_loops: Option<u32>,
    pub autonomy: Option<xherd_core::autonomy::Autonomy>,
    pub permission_mode: Option<xherd_core::permission::PermissionMode>,
    pub default_runtime: Option<String>,
    /// Runtime → model (replaces the map; an empty model removes the entry).
    pub runtime_models: Option<std::collections::BTreeMap<String, String>>,
    pub open_in: Option<xherd_core::config::OpenIn>,
    pub max_auto_approvals_in_row: Option<u32>,
    #[serde(alias = "agentTokenBudget")]
    pub job_token_budget: Option<u64>,
    pub daily_token_budget: Option<u64>,
    pub max_diff_lines: Option<usize>,
    /// Terminal uploads (#192): bytes per file (at most 30 MiB, not above the
    /// per-job bytes), bytes per job, files per job.
    pub upload_max_bytes: Option<u64>,
    pub upload_max_job_bytes: Option<u64>,
    pub upload_max_files: Option<usize>,
    /// Replaces the list of sensitive path globs.
    pub sensitive_paths: Option<Vec<String>>,
    /// Tool → default version; an empty version removes the tool's default.
    pub toolchain_defaults: Option<std::collections::BTreeMap<String, String>>,
    /// Appearance (#202, R13): what a job page opens on.
    pub ui: Option<xherd_core::config::UiSettings>,
    /// Seconds a permission request waits for an answer (30 to 3600).
    pub permission_wait_sec: Option<u64>,
    /// Replaces the plugin catalogue sources (`github` or `https://` index
    /// URLs; at most 20, R1).
    pub plugin_catalogs: Option<Vec<String>>,
}

/// Most catalogue sources a save accepts.
const MAX_CATALOGS: usize = 20;

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

/// Schemes a user link may not use (R-706): web pages and script or file
/// URLs are not "open in an app", and `xherd-term` is the handler's own.
const REFUSED_LINK_SCHEMES: &[&str] = &[
    "http",
    "https",
    "ws",
    "wss",
    "ftp",
    "javascript",
    "data",
    "vbscript",
    "blob",
    "file",
    "about",
    "xherd-term",
];

/// A user link URL: at most 2048 bytes, no whitespace or control
/// characters, a scheme (`^[a-z][a-z0-9+.-]*:`, any case) that is not refused.
fn link_url_error(url: &str) -> Option<&'static str> {
    if url.len() > 2048 {
        return Some("is over 2048 bytes");
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Some("has whitespace or control characters");
    }
    let scheme = url.split_once(':').map(|(s, _)| s).unwrap_or("");
    let valid = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
    if !valid {
        return Some("needs a scheme, like idea://");
    }
    REFUSED_LINK_SCHEMES
        .contains(&scheme.to_ascii_lowercase().as_str())
        .then_some("uses a scheme that is not allowed for app links")
}

/// Settings → Open in (R-706): link ids are plain and unique, since they
/// become entry ids (`link:<id>`); every link has a label and an allowed URL.
/// `droppedCommands` can only shrink: a client may dismiss, never add.
fn check_open_in(
    mut o: xherd_core::config::OpenIn,
    old: &xherd_core::config::OpenIn,
) -> Result<xherd_core::config::OpenIn, ActionError> {
    use xherd_core::config::TERMINAL_APPS;
    if let Some(t) = &o.terminal
        && !TERMINAL_APPS.contains(&t.as_str())
    {
        return Err(bad(format!(
            "openIn.terminal must be one of {}",
            TERMINAL_APPS.join(", ")
        )));
    }
    o.hidden.retain(|h| !h.trim().is_empty());
    if o.dropped_commands
        .iter()
        .any(|d| !old.dropped_commands.contains(d))
    {
        return Err(bad("openIn.droppedCommands can only be cleared"));
    }
    let mut ids = std::collections::BTreeSet::new();
    for l in &mut o.links {
        l.label = l.label.trim().to_string();
        l.url = l.url.trim().to_string();
        let plain = !l.id.is_empty()
            && l.id.len() <= 40
            && l.id
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-');
        if !plain || !ids.insert(l.id.clone()) {
            return Err(bad(format!(
                "openIn link id {:?}: lower-case letters, digits and -, unique",
                l.id
            )));
        }
        if l.label.is_empty() {
            return Err(bad(format!("openIn link {}: a label is needed", l.id)));
        }
        if let Some(e) = link_url_error(&l.url) {
            return Err(bad(format!("openIn link {}: the URL {e}", l.id)));
        }
    }
    Ok(o)
}

/// Apply a patch with range checks. Pure, so it is tested directly.
pub fn apply(c: &mut Config, p: ConfigPatch) -> Result<(), ActionError> {
    /// The runner moves a file as one hex frame of at most 64 MiB (#192).
    const UPLOAD_CEILING: u64 = 30 * 1024 * 1024;
    let file = p.upload_max_bytes.unwrap_or(c.upload_max_bytes);
    let job = p.upload_max_job_bytes.unwrap_or(c.upload_max_job_bytes);
    let count = p.upload_max_files.unwrap_or(c.upload_max_files);
    if file == 0 || job == 0 || count == 0 {
        return Err(bad(
            "uploadMaxBytes, uploadMaxJobBytes and uploadMaxFiles must be at least 1",
        ));
    }
    if file > UPLOAD_CEILING {
        return Err(bad("uploadMaxBytes must be at most 30 MiB"));
    }
    if file > job {
        return Err(bad("uploadMaxBytes must not exceed uploadMaxJobBytes"));
    }
    c.upload_max_bytes = file;
    c.upload_max_job_bytes = job;
    c.upload_max_files = count;
    if let Some(n) = p.max_concurrent_jobs {
        if !(1..=64).contains(&n) {
            return Err(bad("maxConcurrentJobs must be 1 to 64"));
        }
        c.max_concurrent_jobs = n;
    }
    if let Some(n) = p.pause_timeout_sec {
        if !(5..=3600).contains(&n) {
            return Err(bad("pauseTimeoutSec must be 5 to 3600"));
        }
        c.pause_timeout_sec = n;
    }
    if let Some(u) = p.ui {
        c.ui = u;
    }
    if let Some(n) = p.permission_wait_sec {
        if !(30..=3600).contains(&n) {
            return Err(bad("permissionWaitSec must be 30 to 3600"));
        }
        c.permission_wait_sec = n;
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
    if let Some(m) = p.permission_mode {
        c.permission_mode = m;
    }
    if let Some(r) = p.default_runtime {
        c.default_runtime = r.trim().to_string();
    }
    if let Some(m) = p.runtime_models {
        c.runtime_models =
            xherd_core::model::clean_map(m).map_err(|e| bad(format!("runtimeModels: {e}")))?;
    }
    if let Some(o) = p.open_in {
        c.open_in = check_open_in(o, &c.open_in)?;
    }
    if let Some(n) = p.max_auto_approvals_in_row {
        c.max_auto_approvals_in_row = n;
    }
    if let Some(n) = p.job_token_budget {
        c.job_token_budget = n;
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
    if let Some(list) = p.plugin_catalogs {
        let mut out: Vec<String> = Vec::new();
        for s in list {
            let s = s.trim();
            if s.is_empty() || out.iter().any(|o| o == s) {
                continue;
            }
            if let Err(why) = xherd_plugin::catalog::Source::parse(s) {
                return Err(ActionError {
                    code: "bad_catalog_url",
                    message: format!("pluginCatalogs: {why}"),
                });
            }
            out.push(s.to_string());
        }
        if out.len() > MAX_CATALOGS {
            return Err(ActionError {
                code: "bad_catalog_url",
                message: format!("pluginCatalogs: at most {MAX_CATALOGS} sources"),
            });
        }
        c.plugin_catalogs = out;
    }
    if let Some(v) = p.ntfy_server {
        let v = v.trim().trim_end_matches('/');
        if !v.is_empty() && !crate::notifier::valid_server(v) {
            return Err(bad("ntfyServer must be an http(s) URL without spaces"));
        }
        c.ntfy_server = (!v.is_empty()).then(|| v.to_string());
    }
    if let Some(list) = p.muted_notifications {
        if let Some(k) = list
            .iter()
            .find(|k| !xherd_core::config::NOTIFY_KINDS.contains(&k.as_str()))
        {
            return Err(bad(format!("{k:?} is not a notification kind")));
        }
        let mut list = list;
        list.sort();
        list.dedup();
        c.muted_notifications = list;
    }
    if let Some(b) = p.desktop_notifications {
        c.desktop_notifications = b;
    }
    if let Some(b) = p.suggest_agents {
        c.suggest_agents = b;
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
    pub fn saved_config(&self) -> Result<Config, ActionError> {
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
        // Everything but these applies live (`save_config`); `running` is what
        // the daemon uses now, so it takes the saved value of each live field.
        let mut running = (*self.config).clone();
        running.apply_live_from(&saved);
        Ok(json!({
            "config": shown(&saved),
            "running": shown(&running),
            "restartNeeded": saved != running || self.plugin_changes(),
        }))
    }

    /// Names must exist, and the last plugin of a kind that has one on cannot be
    /// switched off, except for the optional kinds (trackers, Git providers,
    /// Herd views). A kind with none on, as after a first run (everything
    /// starts off), stays that way until the user picks: `before` is the list
    /// saved so far.
    fn check_disabled(&self, off: &[String], before: &[String]) -> Result<(), ActionError> {
        if let Some(n) = off.iter().find(|n| self.plugins.get(n).is_none()) {
            return Err(bad(format!("no plugin named {n}")));
        }
        let kinds: std::collections::BTreeSet<String> = self
            .plugins
            .all()
            .map(|p| format!("{:?}", p.manifest.kind))
            .collect();
        for k in kinds {
            // Having no issue tracker, Git provider or view plugin is fine
            // (Git access works without a provider, the Herd has its own
            // views); every other kind needs one.
            if k == "Tracker" || k == "Host" || k == "View" {
                continue;
            }
            let on_in = |list: &[String]| {
                self.plugins
                    .all()
                    .filter(|p| format!("{:?}", p.manifest.kind) == k)
                    .any(|p| !list.iter().any(|n| n == p.name()))
            };
            if on_in(before) && !on_in(off) {
                return Err(bad(format!(
                    "at least one {} plugin must stay on",
                    k.to_lowercase()
                )));
            }
        }
        Ok(())
    }

    /// `name` is a runtime plugin that is installed and not in `off`.
    pub(crate) fn check_runtime(&self, name: &str, off: &[String]) -> Result<(), String> {
        match self.plugins.get(name) {
            Some(p) if p.manifest.kind != xherd_plugin::Kind::Runtime => {
                Err(format!("{name} is not a runtime plugin"))
            }
            Some(_) if off.iter().any(|n| n == name) => Err(format!("{name} is switched off")),
            Some(_) => Ok(()),
            None => Err(format!("no runtime plugin named {name}")),
        }
    }

    /// `name` is a runtime plugin that can choose a model (switched off is
    /// allowed: its default model is kept for when it is on again).
    pub(crate) fn check_model_runtime(&self, name: &str) -> Result<(), String> {
        self.check_runtime(name, &[])?;
        let p = self.plugins.get(name).expect("checked above");
        match xherd_plugin::runtime::Terminal::of(&p.manifest) {
            Ok(t) if !t.model_args.is_empty() => Ok(()),
            Ok(_) => Err(format!("{name} cannot choose a model")),
            Err(e) => Err(e),
        }
    }

    pub fn save_config(&self, patch: ConfigPatch) -> Result<Value, ActionError> {
        // Held from reading the config to saving it (`set_git_hosts` too).
        let write = self.config_write.lock().unwrap_or_else(|e| e.into_inner());
        let mut c = self.saved_config()?;
        let was_disabled = c.disabled_plugins.clone();
        let sets_default = patch.default_runtime.is_some();
        apply(&mut c, patch)?;
        self.check_disabled(&c.disabled_plugins, &was_disabled)?;
        // A first run has every plugin off, the default runtime included:
        // when the default is off and the patch did not choose one, take the
        // first runtime that is on, and keep it while none is.
        let on = |p: &&xherd_plugin::Plugin| !c.disabled_plugins.iter().any(|n| n == p.name());
        let first_on = self
            .plugins
            .of_kind(xherd_plugin::Kind::Runtime)
            .find(on)
            .map(|p| p.name().to_string());
        if let Some(name) = &first_on {
            if !sets_default && c.disabled_plugins.contains(&c.default_runtime) {
                c.default_runtime = name.clone();
            }
            self.check_runtime(&c.default_runtime, &c.disabled_plugins)
                .map_err(|e| bad(format!("defaultRuntime: {e}")))?;
        }
        for rt in c.runtime_models.keys() {
            self.check_model_runtime(rt)
                .map_err(|e| bad(format!("runtimeModels: {e}")))?;
        }
        c.save(&self.paths.config()).map_err(|e| ActionError {
            code: "io",
            message: e.to_string(),
        })?;
        drop(write);
        // A runtime switched on installs its CLI (R3); this only spawns.
        for name in was_disabled
            .iter()
            .filter(|n| !c.disabled_plugins.contains(n))
        {
            self.ensure_runtime(name);
        }
        // Settings the actors read apply at once (the rest needs a restart).
        self.settings.set(ActorSettings::from_config(
            &c,
            (*self.settings.get()).clone(),
            self.pr_poll_override,
        ));
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
            "maxConcurrentJobs": 6, "freshSessions": false, "workspace": "/w",
            "pluginSettings": {"github": {"gh": "/bin/gh"}}, "welcomed": true
        }))
        .unwrap();
        apply(&mut c, p).unwrap();
        assert_eq!(c.max_concurrent_jobs, 6);
        assert!(!c.fresh_sessions && c.welcomed);
        assert_eq!(c.workspace.as_deref(), Some("/w"));
        assert_eq!(c.plugin_settings["github"]["gh"], "/bin/gh");

        let p: ConfigPatch = serde_json::from_value(json!({"workspace": ""})).unwrap();
        apply(&mut c, p).unwrap();
        assert_eq!(c.workspace, None);

        for bad in [
            json!({"maxConcurrentJobs": 0}),
            json!({"workspace": "rel/dir"}),
            json!({"pluginSettings": {"x": 1}}),
        ] {
            let p: ConfigPatch = serde_json::from_value(bad).unwrap();
            assert_eq!(apply(&mut c, p).unwrap_err().code, "bad_config");
        }
        assert!(serde_json::from_value::<ConfigPatch>(json!({"port": 1})).is_err());
        assert!(serde_json::from_value::<ConfigPatch>(json!({"userToken": "x"})).is_err());
    }

    /// #202 R13: `ui.jobView` and `permissionWaitSec` are patched and checked.
    #[test]
    fn ui_and_permission_wait_are_patched() {
        let patch = |v: Value| -> ConfigPatch { serde_json::from_value(v).unwrap() };
        let mut c = Config::default();
        apply(
            &mut c,
            patch(json!({"ui": {"jobView": "terminal"}, "permissionWaitSec": 60})),
        )
        .unwrap();
        assert_eq!(c.ui.job_view, xherd_core::config::JobView::Terminal);
        assert_eq!(c.permission_wait_sec, 60);
        for bad in [
            json!({"permissionWaitSec": 29}),
            json!({"permissionWaitSec": 3601}),
        ] {
            assert_eq!(apply(&mut c, patch(bad)).unwrap_err().code, "bad_config");
        }
        assert_eq!(c.permission_wait_sec, 60);
        assert!(serde_json::from_value::<ConfigPatch>(json!({"ui": {"jobView": "x"}})).is_err());
    }

    /// #192 R3a: the upload limits apply together and refuse nonsense.
    #[test]
    fn upload_limits_are_checked() {
        const MIB: u64 = 1024 * 1024;
        let patch = |v: Value| -> ConfigPatch { serde_json::from_value(v).unwrap() };
        let mut c = Config::default();
        apply(
            &mut c,
            patch(json!({"uploadMaxBytes": 10 * MIB, "uploadMaxJobBytes": 100 * MIB, "uploadMaxFiles": 5})),
        )
        .unwrap();
        assert_eq!(
            (
                c.upload_max_bytes,
                c.upload_max_job_bytes,
                c.upload_max_files
            ),
            (10 * MIB, 100 * MIB, 5)
        );
        // Lowering the per-job bytes below the per-file bytes in one patch,
        // or against the saved value, is refused and changes nothing.
        for bad in [
            json!({"uploadMaxBytes": 0}),
            json!({"uploadMaxJobBytes": 0}),
            json!({"uploadMaxFiles": 0}),
            json!({"uploadMaxBytes": 31 * MIB, "uploadMaxJobBytes": 500 * MIB}),
            json!({"uploadMaxBytes": 20 * MIB, "uploadMaxJobBytes": 10 * MIB}),
            json!({"uploadMaxJobBytes": 5 * MIB}),
        ] {
            assert_eq!(apply(&mut c, patch(bad)).unwrap_err().code, "bad_config");
        }
        assert_eq!(
            (
                c.upload_max_bytes,
                c.upload_max_job_bytes,
                c.upload_max_files
            ),
            (10 * MIB, 100 * MIB, 5)
        );
        apply(&mut c, patch(json!({"uploadMaxBytes": 30 * MIB}))).unwrap();
        assert_eq!(c.upload_max_bytes, 30 * MIB);
    }

    #[test]
    fn muted_notification_kinds_are_checked_and_sorted() {
        let mut c = Config::default();
        let p: ConfigPatch = serde_json::from_value(
            json!({"mutedNotifications": ["idle", "gate", "idle"], "desktopNotifications": true}),
        )
        .unwrap();
        apply(&mut c, p).unwrap();
        assert_eq!(c.muted_notifications, ["gate", "idle"]);
        assert!(c.desktop_notifications);
        let p: ConfigPatch =
            serde_json::from_value(json!({"mutedNotifications": ["nope"]})).unwrap();
        assert_eq!(apply(&mut c, p).unwrap_err().code, "bad_config");
    }

    fn open_in_patch(v: Value) -> ConfigPatch {
        serde_json::from_value(json!({"openIn": v})).unwrap()
    }

    fn link(url: &str) -> Value {
        json!({"links": [{"id": "a", "label": "A", "url": url}]})
    }

    #[test]
    fn open_in_links_are_checked() {
        let mut c = Config::default();
        apply(
            &mut c,
            open_in_patch(json!({"hidden": ["zed", " "], "terminal": "iterm",
                "links": [{"id": "idea", "label": " IDEA ", "url": " idea://open?file={dir} "},
                          {"id": "jb", "label": "Toolbox", "url": "jetbrains://idea/navigate?path={dir}"}]})),
        )
        .unwrap();
        assert_eq!(c.open_in.hidden, ["zed"]);
        assert_eq!(c.open_in.links[0].label, "IDEA");
        assert_eq!(c.open_in.links[0].url, "idea://open?file={dir}");
        for bad in [
            json!({"terminal": "st"}),
            json!({"links": [{"id": "Idea", "label": "x", "url": "idea://x"}]}),
            json!({"links": [{"id": "a", "label": "x", "url": "a://x"}, {"id": "a", "label": "y", "url": "b://y"}]}),
            json!({"links": [{"id": "a", "label": " ", "url": "a://x"}]}),
        ] {
            assert_eq!(
                apply(&mut c, open_in_patch(bad)).unwrap_err().code,
                "bad_config"
            );
        }
        let extra =
            json!({"openIn": {"links": [{"id": "a", "label": "x", "url": "a://x", "argv": []}]}});
        assert!(serde_json::from_value::<ConfigPatch>(extra).is_err());
    }

    #[test]
    fn open_in_link_urls_are_checked() {
        let mut c = Config::default();
        for scheme in super::REFUSED_LINK_SCHEMES {
            for s in [scheme.to_string(), scheme.to_uppercase()] {
                let url = format!("{s}://example.com/{{dir}}");
                let err = apply(&mut c, open_in_patch(link(&url))).unwrap_err();
                assert_eq!(err.code, "bad_config", "{url}");
            }
        }
        let long = format!("idea://{}", "x".repeat(2048));
        for url in [
            "idea",
            "/open?file={dir}",
            "1idea://x",
            "id ea://x",
            "idea://open?file={dir} x",
            "idea://x\ty",
            "idea://x\u{7}",
            "",
            &long,
        ] {
            let err = apply(&mut c, open_in_patch(link(url))).unwrap_err();
            assert_eq!(err.code, "bad_config", "{url:?}");
        }
        apply(&mut c, open_in_patch(link("Vs-Code+x.1://open"))).unwrap();
    }

    #[test]
    fn dropped_commands_can_only_shrink() {
        let mut c = Config::default();
        c.open_in.dropped_commands = vec!["Fork".into(), "Tower".into()];
        let grow = open_in_patch(json!({"droppedCommands": ["Fork", "Other"]}));
        assert_eq!(apply(&mut c, grow).unwrap_err().code, "bad_config");
        apply(&mut c, open_in_patch(json!({"droppedCommands": ["Tower"]}))).unwrap();
        assert_eq!(c.open_in.dropped_commands, ["Tower"]);
        apply(&mut c, open_in_patch(json!({}))).unwrap();
        assert!(c.open_in.dropped_commands.is_empty());
        let back = open_in_patch(json!({"droppedCommands": ["Tower"]}));
        assert_eq!(apply(&mut c, back).unwrap_err().code, "bad_config");
    }

    #[test]
    fn plugin_catalogs_are_cleaned_and_validated() {
        let patch = |v: Value| -> ConfigPatch {
            serde_json::from_value(json!({ "pluginCatalogs": v })).unwrap()
        };
        let mut c = Config::default();
        apply(
            &mut c,
            patch(json!([
                " github ",
                "",
                "https://example.com/c.json",
                "github"
            ])),
        )
        .unwrap();
        assert_eq!(c.plugin_catalogs, ["github", "https://example.com/c.json"]);
        apply(&mut c, patch(json!([]))).unwrap();
        assert!(c.plugin_catalogs.is_empty());

        let many: Vec<String> = (0..21).map(|i| format!("https://e.com/{i}")).collect();
        for bad in [
            json!(["http://example.com/c.json"]),
            json!(["github", "ftp://x"]),
            json!(many),
        ] {
            let err = apply(&mut c, patch(bad)).unwrap_err();
            assert_eq!(err.code, "bad_catalog_url");
        }
        assert!(
            c.plugin_catalogs.is_empty(),
            "a refused save changes nothing"
        );
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
