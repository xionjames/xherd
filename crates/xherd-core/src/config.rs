//! `config.json` with a schema version, migrations, and unknown keys preserved (R-1306).

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::paths::write_atomic;

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

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
    pub max_concurrent_agents: usize,
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
    /// Workspace root holding `worktrees/` (design §8.1). `None`:
    /// `$XHERD_WORKSPACE`, else `~/xherd`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// tmux socket name (`tmux -L`); `None` uses the default server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tmux_socket: Option<String>,
    /// Plugin settings by plugin name (non-secret).
    pub plugin_settings: Map<String, Value>,
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
            max_concurrent_agents: 4,
            gate_park_minutes: 30,
            idle_alert_minutes: 15,
            pause_timeout_sec: 120,
            fresh_sessions: true,
            event_retention_days: 90,
            update_check: true,
            workspace: None,
            tmux_socket: None,
            plugin_settings: Map::new(),
            extra: Map::new(),
        }
    }
}

/// Migrations from version N to N+1, applied to the raw JSON. Index 0 = 0 → 1.
type Migration = fn(&mut Map<String, Value>);
const MIGRATIONS: &[Migration] = &[|_v0| {}];

impl Config {
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
        let cfg = serde_json::from_value(Value::Object(map))
            .map_err(|source| ConfigError::Json { path: p, source })?;
        Ok((cfg, migrated))
    }

    /// Atomic save, mode 0600 (it holds the user token).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        write_atomic(path, &bytes, Some(0o600))
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
