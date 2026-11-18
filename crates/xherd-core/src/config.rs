//! `config.json` with a schema version, migrations, and unknown keys preserved (R-1306).

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::paths::write_atomic;

pub const CONFIG_SCHEMA_VERSION: u32 = 4;

/// The runtime of agents created before they named one, and the default
/// `defaultRuntime`.
pub const DEFAULT_RUNTIME: &str = "claude-code";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("{path} is not valid JSON: {source}")]
    Json {
        path: String,
        source: serde_json::Error,
    },
    #[error(
        "{path} has schemaVersion {found}, newer than this xherd supports ({supported}); update xherd"
    )]
    TooNew {
        path: String,
        found: u32,
        supported: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub schema_version: u32,
    pub port: u16,
    pub bind: String,
    /// Token of the local user (CLI, local browser). Generated on first start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_token: Option<String>,
    pub max_concurrent_jobs: usize,
    /// 0 = never park.
    pub gate_park_minutes: u64,
    /// 0 = no idle alerts.
    pub idle_alert_minutes: u64,
    pub pause_timeout_sec: u64,
    /// After an approved gate, replace the runtime session with a new one
    /// started with the next command instead of typing it (R-023).
    pub fresh_sessions: bool,
    pub event_retention_days: u64,
    pub update_check: bool,
    /// The first-run Welcome screen was completed or skipped.
    pub welcomed: bool,
    /// Workspace root holding `worktrees/` (design §8.1). `None`:
    /// `$XHERD_WORKSPACE`, else `~/xherd`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// tmux socket name (`tmux -L`); `None` uses the default server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tmux_socket: Option<String>,
    /// Plugin settings by plugin name (non-secret).
    pub plugin_settings: Map<String, Value>,
    /// Plugins switched off in Welcome/Settings; read live, no restart.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub disabled_plugins: Vec<String>,
    /// Commit identity for the managed git config (agents commit as this
    /// user). Both are needed; saved live, no restart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_user_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_user_email: Option<String>,
    /// Identities by remote (`github.com/my-company`), over the global one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub git_identities: Vec<GitIdentity>,
    /// A session running longer than this many minutes is ended and the agent
    /// paused (R-1206). 0 = off.
    pub max_session_minutes: u64,
    /// Minutes between polls of a watched pull request (R-303); 0 = no polling.
    pub pr_poll_minutes: u64,
    /// Review→fix rounds after which the review gate is held for a human
    /// (R-120 `maxReviewLoops`). 0 = no limit. Global until profiles.
    pub max_review_loops: u32,
    /// Autonomy of new agents (R-109); global until profiles. `never` is not
    /// selectable yet.
    pub autonomy: crate::autonomy::Autonomy,
    /// Tool-permission mode of new agents (R-002a); a profile or the request
    /// overrides it.
    pub permission_mode: crate::permission::PermissionMode,
    /// Runtime plugin of new agents (`claude-code`, `opencode`, `codex`); a profile or
    /// the request overrides it. Also the runtime of TODO condition checks.
    pub default_runtime: String,
    /// Runtime plugin → model of new agents (issue #112); a profile's
    /// `models` or the request overrides it. No entry = the runtime's own
    /// default. Live.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub runtime_models: std::collections::BTreeMap<String, String>,
    /// Settings → Open in (R-706, R-707).
    pub open_in: OpenIn,
    /// Consecutive automatic approvals after which the next gate waits for a
    /// human (R-120). 0 = no limit.
    pub max_auto_approvals_in_row: u32,
    /// Token budget of one agent and of all agents per UTC day (R-1205). At
    /// 80 % a notification; at 100 % the agent is paused at its next stop.
    /// 0 = no budget.
    pub job_token_budget: u64,
    pub daily_token_budget: u64,
    /// Code gates hold a commit proposal with more changed lines than this
    /// (R-115 `maxDiffLines`). Global until repo profiles exist.
    pub max_diff_lines: usize,
    /// Globs of sensitive paths: a commit proposal touching one is held
    /// (R-115). `*` stays in a folder, `**` crosses folders, a pattern
    /// without `/` matches the file name anywhere. Global until profiles.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sensitive_paths: Vec<String>,
    /// Tool name → version that agents get when the worktree has no version
    /// file for it (R-653). Read at daemon start.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub toolchain_defaults: std::collections::BTreeMap<String, String>,
    /// ntfy server (`https://ntfy.example.com`) and topic that notifications
    /// are also sent to (spec §16.3). Both are needed; off until set. Read
    /// live, no restart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ntfy_server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ntfy_topic: Option<String>,
    /// Notification kinds the user does not want at all (bell, browser, ntfy
    /// and desktop); see `NOTIFY_KINDS`. Live.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub muted_notifications: Vec<String>,
    /// Also show notifications as operating system notifications from the
    /// daemon (`osascript` / `notify-send`); native installs only. Live.
    pub desktop_notifications: bool,
    /// Offer to save a job's setup as an agent (R30); "Don't suggest" turns
    /// it off. Live.
    pub suggest_agents: bool,
    /// Agents may register job actions (`xherd action add`, R30); off refuses
    /// `action.add` with `actions_off`. Live.
    pub job_actions: bool,
    /// Most actions one job holds, profile actions included (R6, R30). Live.
    pub max_actions_per_job: usize,
    /// Git access per host (R-685). Tokens live in `secrets.json` under `git`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub git_hosts: Vec<GitHost>,
    /// Keys this version does not know, kept on save.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            schema_version: CONFIG_SCHEMA_VERSION,
            port: 4740,
            bind: "127.0.0.1".into(),
            user_token: None,
            max_concurrent_jobs: 4,
            gate_park_minutes: 30,
            idle_alert_minutes: 15,
            pause_timeout_sec: 120,
            fresh_sessions: true,
            event_retention_days: 90,
            update_check: true,
            welcomed: false,
            workspace: None,
            tmux_socket: None,
            plugin_settings: Map::new(),
            disabled_plugins: Vec::new(),
            git_user_name: None,
            git_user_email: None,
            git_identities: Vec::new(),
            git_hosts: Vec::new(),
            ntfy_server: None,
            ntfy_topic: None,
            muted_notifications: Vec::new(),
            desktop_notifications: false,
            suggest_agents: true,
            job_actions: true,
            max_actions_per_job: 12,
            max_session_minutes: 0,
            pr_poll_minutes: 5,
            max_review_loops: 3,
            autonomy: crate::autonomy::Autonomy::Ask,
            permission_mode: Default::default(),
            default_runtime: DEFAULT_RUNTIME.into(),
            runtime_models: Default::default(),
            open_in: OpenIn::default(),
            max_auto_approvals_in_row: 10,
            job_token_budget: 0,
            daily_token_budget: 0,
            max_diff_lines: 1000,
            sensitive_paths: Vec::new(),
            toolchain_defaults: Default::default(),
            extra: Map::new(),
        }
    }
}

/// Settings → Open in (R-706, R-707): entries left out of the menu, the
/// terminal the `xherd-term` link handler opens, and the user's own links.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenIn {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<OpenLink>,
    /// Labels of the schema 2 commands the migration removed, shown in
    /// Settings until the user dismisses them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dropped_commands: Vec<String>,
}

/// A user link (R-706): a URL the browser opens, with `{dir}` and `{agent}`
/// replaced (percent-encoded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenLink {
    pub id: String,
    pub label: String,
    pub url: String,
}

/// Terminal apps the `xherd-term` link handler knows how to open (R-707):
/// macOS's `terminal` and `iterm`, `ghostty` on both, the rest on Linux.
pub const TERMINAL_APPS: &[&str] = &[
    "terminal",
    "iterm",
    "ghostty",
    "konsole",
    "gnome-terminal",
    "kitty",
    "alacritty",
    "wezterm",
    "xfce4-terminal",
    "foot",
    "xterm",
    "xdg-terminal-exec",
    "x-terminal-emulator",
];

/// Where git gets an HTTPS password for a host (R-684). Never a URL or
/// `.git/config`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Credential {
    /// git's own configuration decides.
    #[default]
    None,
    /// The host's git provider supplies the helper (`gh auth git-credential`
    /// for GitHub, D24); before schema 2 this was `gh` / `glab`.
    Provider,
    /// `xherd credential`: the token stored in xherd's secrets.
    Xherd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitHost {
    /// `github.com`, `gitlab.example.com:8443`.
    pub host: String,
    /// `https`, `ssh`: what the doctor checks.
    #[serde(default = "both_protocols")]
    pub protocols: Vec<String>,
    #[serde(default)]
    pub credential: Credential,
    /// The `host` plugin that knows this host's API, token user and login
    /// (D24, R-689). Absent: the first provider claiming the host name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// CA bundle for this host's HTTPS (`http.<url>.sslCAInfo`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_file: Option<String>,
    /// Rewrite SSH remotes of this host to HTTPS (off by default).
    #[serde(default)]
    pub prefer_https: bool,
    /// Rewrite HTTPS remotes of this host to SSH (off by default).
    #[serde(default)]
    pub prefer_ssh: bool,
}

/// Commit identity for repositories whose remote is under `scope`
/// (`github.com/my-company`, or a whole host). Beats the global identity;
/// matched by git itself (`includeIf hasconfig:remote.*.url`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitIdentity {
    pub scope: String,
    pub name: String,
    pub email: String,
}

fn both_protocols() -> Vec<String> {
    vec!["https".into(), "ssh".into()]
}

/// Migrations from version N to N+1, applied to the raw JSON. Index 0 = 0 → 1.
type Migration = fn(&mut Map<String, Value>);
const MIGRATIONS: &[Migration] = &[
    |_v0| {},
    git_hosts_to_providers,
    open_in_commands_to_links,
    agent_keys_to_job_keys,
];

/// 3 → 4 (#141): `maxConcurrentAgents` and `agentTokenBudget` become
/// `maxConcurrentJobs` and `jobTokenBudget`; a new key already present wins,
/// the old one is never written again.
fn agent_keys_to_job_keys(m: &mut Map<String, Value>) {
    for (old, new) in [
        ("maxConcurrentAgents", "maxConcurrentJobs"),
        ("agentTokenBudget", "jobTokenBudget"),
    ] {
        if let Some(v) = m.remove(old) {
            m.entry(new).or_insert(v);
        }
    }
}

/// 1 → 2 (R-693): a host's `gh` / `glab` credential becomes the provider
/// credential bound to `github` / `gitlab`; `xherd` and `none` stay.
fn git_hosts_to_providers(m: &mut Map<String, Value>) {
    let Some(Value::Array(hosts)) = m.get_mut("gitHosts") else {
        return;
    };
    for h in hosts.iter_mut().filter_map(Value::as_object_mut) {
        let provider = match h.get("credential").and_then(Value::as_str) {
            Some("gh") => "github",
            Some("glab") => "gitlab",
            _ => continue,
        };
        h.insert("credential".into(), "provider".into());
        h.insert("provider".into(), provider.into());
    }
}

/// 2 → 3 (R-706): the companion's commands are gone. Their labels stay in
/// `droppedCommands` so Settings can say so, and the menu ids that named
/// companion entries (`cmd:*`, `companion:*`) leave `hidden`.
fn open_in_commands_to_links(m: &mut Map<String, Value>) {
    let Some(Value::Object(o)) = m.get_mut("openIn") else {
        return;
    };
    if let Some(Value::Array(cmds)) = o.remove("commands") {
        let labels: Vec<Value> = cmds
            .iter()
            .filter_map(|c| c.get("label").and_then(Value::as_str))
            .map(Value::from)
            .collect();
        if !labels.is_empty() {
            o.insert("droppedCommands".into(), labels.into());
        }
    }
    if let Some(Value::Array(hidden)) = o.get_mut("hidden") {
        hidden.retain(|h| {
            h.as_str()
                .is_none_or(|h| !h.starts_with("cmd:") && !h.starts_with("companion:"))
        });
    }
}

/// The kinds of notification (spec §16.3), what `mutedNotifications` may name.
pub const NOTIFY_KINDS: &[&str] = &[
    "gate",
    "blocked",
    "idle",
    "delivery",
    "error",
    "recovery",
    "budget",
    "limit",
    "pr",
    "pr-comments",
    "todo",
    "suggestion",
];

impl Config {
    /// Copies the settings the daemon applies while running (everything but
    /// the port, bind address, concurrency, workspace, tmux socket and plugin
    /// settings) from `saved`, so the rest can be compared for a restart.
    pub fn apply_live_from(&mut self, saved: &Config) {
        let s = saved.clone();
        self.gate_park_minutes = s.gate_park_minutes;
        self.idle_alert_minutes = s.idle_alert_minutes;
        self.pause_timeout_sec = s.pause_timeout_sec;
        self.fresh_sessions = s.fresh_sessions;
        self.event_retention_days = s.event_retention_days;
        self.welcomed = s.welcomed;
        self.disabled_plugins = s.disabled_plugins;
        self.git_user_name = s.git_user_name;
        self.git_user_email = s.git_user_email;
        self.git_identities = s.git_identities;
        self.max_session_minutes = s.max_session_minutes;
        self.pr_poll_minutes = s.pr_poll_minutes;
        self.max_review_loops = s.max_review_loops;
        self.autonomy = s.autonomy;
        self.permission_mode = s.permission_mode;
        self.default_runtime = s.default_runtime;
        self.runtime_models = s.runtime_models;
        self.open_in = s.open_in;
        self.max_auto_approvals_in_row = s.max_auto_approvals_in_row;
        self.job_token_budget = s.job_token_budget;
        self.daily_token_budget = s.daily_token_budget;
        self.max_diff_lines = s.max_diff_lines;
        self.sensitive_paths = s.sensitive_paths;
        self.toolchain_defaults = s.toolchain_defaults;
        self.ntfy_server = s.ntfy_server;
        self.ntfy_topic = s.ntfy_topic;
        self.muted_notifications = s.muted_notifications;
        self.desktop_notifications = s.desktop_notifications;
        self.suggest_agents = s.suggest_agents;
        self.job_actions = s.job_actions;
        self.max_actions_per_job = s.max_actions_per_job;
        self.git_hosts = s.git_hosts;
        self.user_token = s.user_token;
    }

    /// Load, migrating older schema versions. A missing file yields defaults.
    pub fn load(path: &Path) -> Result<(Config, bool), ConfigError> {
        let p = path.display().to_string();
        let raw = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Config::default(), false));
            }
            Err(source) => return Err(ConfigError::Io { path: p, source }),
        };
        let mut map: Map<String, Value> =
            serde_json::from_slice(&raw).map_err(|source| ConfigError::Json {
                path: p.clone(),
                source,
            })?;
        let found = map
            .get("schemaVersion")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        if found > CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::TooNew {
                path: p,
                found,
                supported: CONFIG_SCHEMA_VERSION,
            });
        }
        let migrated = found < CONFIG_SCHEMA_VERSION;
        for m in &MIGRATIONS[found as usize..CONFIG_SCHEMA_VERSION as usize] {
            m(&mut map);
        }
        map.insert("schemaVersion".into(), CONFIG_SCHEMA_VERSION.into());
        let mut cfg: Config = serde_json::from_value(Value::Object(map))
            .map_err(|source| ConfigError::Json { path: p, source })?;
        // A kind that no longer exists (`auto-approved`, #135) is dropped, so
        // saving Settings is not refused over it.
        cfg.muted_notifications
            .retain(|k| NOTIFY_KINDS.contains(&k.as_str()));
        Ok((cfg, migrated))
    }

    /// Atomic save, mode 0600 (it holds the user token).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        write_atomic(path, &bytes, Some(0o600))
    }

    /// The tmux server to use: `$XHERD_TMUX_SOCKET` (the container's, a path)
    /// wins over the saved name.
    pub fn tmux_server(&self) -> Option<String> {
        std::env::var("XHERD_TMUX_SOCKET")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| self.tmux_socket.clone())
    }

    pub fn workspace_root(&self) -> std::path::PathBuf {
        let nonempty = |s: Option<String>| s.filter(|s| !s.is_empty());
        nonempty(self.workspace.clone())
            .or_else(|| nonempty(std::env::var("XHERD_WORKSPACE").ok()))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
                    .join("xherd")
            })
    }

    pub fn url(&self) -> String {
        format!("http://{}:{}", self.bind, self.port)
    }
}

/// Key of git host tokens in `secrets.json` (host → token).
pub const GIT_SECRETS: &str = "git";

/// `secrets.json`: plugin secrets by plugin name, then key. Never sent to the UI.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Secrets(pub Map<String, Value>);

impl Secrets {
    pub fn load(path: &Path) -> std::io::Result<Secrets> {
        match std::fs::read(path) {
            Ok(b) => serde_json::from_slice(&b).map_err(std::io::Error::other),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Secrets::default()),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        write_atomic(path, &bytes, Some(0o600))
    }

    pub fn for_plugin(&self, plugin: &str) -> Map<String, Value> {
        self.0
            .get(plugin)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }

    /// The HTTPS token stored for a git host (Settings → Git access).
    pub fn git_token(&self, host: &str) -> Option<String> {
        self.for_plugin(GIT_SECRETS)
            .get(host)
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    }

    /// Every secret value, for redaction of captured output.
    pub fn values(&self) -> Vec<String> {
        self.0
            .values()
            .filter_map(Value::as_object)
            .flat_map(|m| m.values())
            .filter_map(Value::as_str)
            .filter(|s| s.len() >= 4)
            .map(str::to_string)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let d = tempfile::tempdir().unwrap();
        let (c, migrated) = Config::load(&d.path().join("config.json")).unwrap();
        assert_eq!(c.port, 4740);
        assert!(!migrated);
    }

    #[test]
    fn unknown_keys_survive_and_old_versions_migrate() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("config.json");
        std::fs::write(&f, r#"{"port": 5000, "futureThing": {"a": 1}}"#).unwrap();
        let (c, migrated) = Config::load(&f).unwrap();
        assert!(migrated);
        assert_eq!(c.port, 5000);
        assert_eq!(c.schema_version, CONFIG_SCHEMA_VERSION);
        c.save(&f).unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
        assert_eq!(v["futureThing"]["a"], 1);
    }

    /// R-693: a config written before providers keeps its meaning.
    #[test]
    fn git_hosts_written_before_providers_are_bound() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("config.json");
        std::fs::write(
            &f,
            r#"{"schemaVersion": 1, "gitHosts": [
                {"host": "github.com", "credential": "gh"},
                {"host": "gitlab.example.com", "credential": "glab"},
                {"host": "git.example.com", "credential": "xherd"},
                {"host": "other.example.com"}]}"#,
        )
        .unwrap();
        let (c, migrated) = Config::load(&f).unwrap();
        assert!(migrated);
        let h = &c.git_hosts;
        assert_eq!(
            (h[0].credential, h[0].provider.as_deref()),
            (Credential::Provider, Some("github"))
        );
        assert_eq!(
            (h[1].credential, h[1].provider.as_deref()),
            (Credential::Provider, Some("gitlab"))
        );
        assert_eq!(
            (h[2].credential, h[2].provider.as_deref()),
            (Credential::Xherd, None)
        );
        assert_eq!(
            (h[3].credential, h[3].provider.as_deref()),
            (Credential::None, None)
        );
    }

    /// #135: the `auto-approved` kind is gone; a saved mute of it is dropped.
    #[test]
    fn unknown_muted_kinds_are_dropped() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("config.json");
        std::fs::write(&f, r#"{"mutedNotifications": ["auto-approved", "gate"]}"#).unwrap();
        let (c, _) = Config::load(&f).unwrap();
        assert_eq!(c.muted_notifications, ["gate"]);
    }

    /// R-706: schema 2 commands become dropped labels, their menu ids go.
    #[test]
    fn open_in_commands_are_dropped() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("config.json");
        std::fs::write(
            &f,
            r#"{"schemaVersion": 2, "futureThing": 1, "openIn": {"terminal": "kitty",
                "hidden": ["cmd:x", "companion:idea", "zed"],
                "commands": [{"id": "x", "label": "Fork", "argv": ["fork", "{dir}"]}]}}"#,
        )
        .unwrap();
        let (c, migrated) = Config::load(&f).unwrap();
        assert!(migrated);
        let o = &c.open_in;
        assert!(o.links.is_empty());
        assert_eq!(o.dropped_commands, ["Fork"]);
        assert_eq!(o.hidden, ["zed"]);
        assert_eq!(o.terminal.as_deref(), Some("kitty"));
        c.save(&f).unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
        assert!(v["openIn"].get("commands").is_none());
        assert_eq!(
            (v["schemaVersion"].as_u64(), v["futureThing"].as_u64()),
            (Some(4), Some(1))
        );
    }

    /// #141: the old keys move to the job names; a new key wins.
    #[test]
    fn agent_keys_become_job_keys() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("config.json");
        std::fs::write(
            &f,
            r#"{"schemaVersion": 3, "maxConcurrentAgents": 7, "agentTokenBudget": 5,
                "maxConcurrentJobs": 2}"#,
        )
        .unwrap();
        let (c, migrated) = Config::load(&f).unwrap();
        assert!(migrated);
        assert_eq!((c.max_concurrent_jobs, c.job_token_budget), (2, 5));
        c.save(&f).unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
        assert!(v.get("maxConcurrentAgents").is_none() && v.get("agentTokenBudget").is_none());
    }

    /// R30: defaults, camelCase keys, and both keys are live.
    #[test]
    fn action_keys_default_and_are_live() {
        let c = Config::default();
        assert!(c.job_actions);
        assert_eq!(c.max_actions_per_job, 12);
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["jobActions"], true);
        assert_eq!(v["maxActionsPerJob"], 12);
        let saved: Config =
            serde_json::from_str(r#"{"jobActions": false, "maxActionsPerJob": 3}"#).unwrap();
        let mut running = Config::default();
        running.apply_live_from(&saved);
        assert!(!running.job_actions);
        assert_eq!(running.max_actions_per_job, 3);
    }

    #[test]
    fn newer_schema_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("config.json");
        std::fs::write(&f, r#"{"schemaVersion": 99}"#).unwrap();
        assert!(matches!(Config::load(&f), Err(ConfigError::TooNew { .. })));
    }

    #[test]
    fn secrets_values_for_redaction() {
        let s: Secrets =
            serde_json::from_str(r#"{"github": {"token": "ghp_abc123"}, "x": {"k": "ab"}}"#)
                .unwrap();
        assert_eq!(s.values(), vec!["ghp_abc123".to_string()]);
        assert_eq!(s.for_plugin("github")["token"], "ghp_abc123");
    }
}
