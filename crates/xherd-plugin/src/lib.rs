//! Plugins (spec §10): manifest loading, the registry (built-ins overridden by
//! external plugins, broken plugins listed with their error), settings and
//! secrets resolution, and the command protocol.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use xherd_core::exec::{Cmd, ExecError, run};

pub mod git;
pub mod gitlab;
pub mod host;
pub mod issues;
pub mod manifest;
pub mod runtime;
pub mod toolchain;
pub mod tracker;
pub mod view;

pub use manifest::{Check, Kind, MANIFEST_FILE, Manifest, Setting};

/// Built-in plugins compiled into the binary (R-504), from `plugins/`.
pub fn builtins() -> Vec<Manifest> {
    [
        include_str!("../../../plugins/claude-code/xherd-plugin.json"),
        include_str!("../../../plugins/opencode/xherd-plugin.json"),
        include_str!("../../../plugins/codex/xherd-plugin.json"),
        include_str!("../../../plugins/github/xherd-plugin.json"),
        include_str!("../../../plugins/gitlab/xherd-plugin.json"),
        include_str!("../../../plugins/bitbucket/xherd-plugin.json"),
        include_str!("../../../plugins/jira/xherd-plugin.json"),
        include_str!("../../../plugins/github-issues/xherd-plugin.json"),
        include_str!("../../../plugins/gitlab-issues/xherd-plugin.json"),
        include_str!("../../../plugins/sdkman/xherd-plugin.json"),
        include_str!("../../../plugins/nvm/xherd-plugin.json"),
        include_str!("../../../plugins/uv/xherd-plugin.json"),
        include_str!("../../../plugins/go/xherd-plugin.json"),
        include_str!("../../../plugins/rustup/xherd-plugin.json"),
        include_str!("../../../plugins/herd-tiles/xherd-plugin.json"),
    ]
    .into_iter()
    .map(|json| Manifest::parse(json).expect("built-in plugin manifest is valid (tested)"))
    .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plugin {
    pub manifest: Manifest,
    /// Plugin folder; `None` for built-ins compiled into the binary.
    pub root: Option<PathBuf>,
}

impl Plugin {
    pub fn name(&self) -> &str {
        &self.manifest.name
    }
    pub fn builtin(&self) -> bool {
        self.root.is_none()
    }
}

/// A plugin folder that failed to load (R-507): listed, never usable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Broken {
    pub name: String,
    pub dir: PathBuf,
    pub error: String,
}

#[derive(Debug, Clone, Default)]
pub struct Registry {
    plugins: BTreeMap<String, Plugin>,
    broken: Vec<Broken>,
}

impl Registry {
    /// Built-ins first, then every `<dir>/*/xherd-plugin.json`; an external plugin
    /// with a built-in's name replaces it (R-504).
    pub fn load(builtins: Vec<Manifest>, dir: &Path) -> Registry {
        let mut r = Registry::default();
        for m in builtins {
            r.plugins.insert(
                m.name.clone(),
                Plugin {
                    manifest: m,
                    root: None,
                },
            );
        }
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default();
        dirs.sort();
        for d in dirs {
            if !d.join(MANIFEST_FILE).exists() {
                continue;
            }
            let folder = d
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match Manifest::load(&d) {
                Ok(m) if m.name != folder => r.broken.push(Broken {
                    name: folder,
                    dir: d.clone(),
                    error: format!("manifest name {:?} differs from its folder name", m.name),
                }),
                Ok(m) => {
                    if r.plugins.contains_key(&m.name) {
                        tracing::info!(plugin = %m.name, "external plugin overrides the built-in one");
                    }
                    r.plugins.insert(
                        m.name.clone(),
                        Plugin {
                            manifest: m,
                            root: Some(d),
                        },
                    );
                }
                Err(error) => {
                    tracing::warn!(plugin = %folder, %error, "plugin not loaded");
                    r.broken.push(Broken {
                        name: folder,
                        dir: d,
                        error,
                    });
                }
            }
        }
        r
    }

    pub fn get(&self, name: &str) -> Option<&Plugin> {
        self.plugins.get(name)
    }

    pub fn all(&self) -> impl Iterator<Item = &Plugin> {
        self.plugins.values()
    }

    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Plugin> {
        self.plugins
            .values()
            .filter(move |p| p.manifest.kind == kind)
    }

    pub fn broken(&self) -> &[Broken] {
        &self.broken
    }

    /// Add plugins that failed before loading (e.g. could not be copied
    /// where they run): listed with the others, never usable.
    pub fn with_broken(mut self, more: Vec<Broken>) -> Registry {
        self.broken.extend(more);
        self
    }
}

/// Settings resolution (R-506): manifest default → Settings → profile override
/// (non-secret only). Unknown keys are dropped.
pub fn resolve_settings(
    m: &Manifest,
    global: Option<&Map<String, Value>>,
    profile: Option<&Map<String, Value>>,
) -> Map<String, Value> {
    let mut out = Map::new();
    for s in m.settings.iter().filter(|s| !s.secret) {
        let v = profile
            .and_then(|p| p.get(&s.key))
            .or_else(|| global.and_then(|g| g.get(&s.key)))
            .or(s.default.as_ref());
        if let Some(v) = v {
            out.insert(s.key.clone(), v.clone());
        }
    }
    out
}

/// Only the secrets this plugin declares.
pub fn resolve_secrets(m: &Manifest, stored: &Map<String, Value>) -> Map<String, Value> {
    m.settings
        .iter()
        .filter(|s| s.secret)
        .filter_map(|s| stored.get(&s.key).map(|v| (s.key.clone(), v.clone())))
        .collect()
}

/// Required settings or secrets that have no value.
pub fn missing_required(
    m: &Manifest,
    settings: &Map<String, Value>,
    secrets: &Map<String, Value>,
) -> Vec<String> {
    m.settings
        .iter()
        .filter(|s| s.required)
        .filter(|s| {
            let src = if s.secret { secrets } else { settings };
            src.get(&s.key)
                .is_none_or(|v| v.is_null() || v.as_str() == Some(""))
        })
        .map(|s| s.key.clone())
        .collect()
}

/// Everything a command run needs besides the command itself (R-503, R-505).
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub settings: Map<String, Value>,
    pub secrets: Map<String, Value>,
    /// The agent the call is about, if any.
    pub agent: Option<Value>,
    pub xherd_bin: String,
    pub xherd_url: String,
    pub plugin_token: Option<String>,
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    /// Extra context for `XHERD_CONTEXT_JSON`.
    pub context: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("plugin {plugin} has no {what} {name:?}")]
    Unknown {
        plugin: String,
        what: &'static str,
        name: String,
    },
    #[error("built-in plugin {plugin} runs {name:?} inside xherd, not as a command")]
    Builtin { plugin: String, name: String },
    #[error(transparent)]
    Exec(#[from] ExecError),
    #[error("{plugin} {name} timed out after {secs}s")]
    Timeout {
        plugin: String,
        name: String,
        secs: u64,
    },
    #[error("{plugin} {name} exited with {code:?}: {stderr}")]
    Failed {
        plugin: String,
        name: String,
        code: Option<i32>,
        stderr: String,
    },
    #[error("{plugin} {name} printed invalid JSON: {error}")]
    BadJson {
        plugin: String,
        name: String,
        error: String,
    },
    #[error("{plugin} {name} returned the wrong shape: {error}")]
    Shape {
        plugin: String,
        name: String,
        error: String,
    },
}

fn build_cmd(p: &Plugin, argv: &[String], ctx: &Context, stdin: &Value, timeout: Duration) -> Cmd {
    let root = p.root.clone().expect("external plugin");
    let mut argv = argv.to_vec();
    if let Some(rel) = argv[0].strip_prefix("./") {
        argv[0] = root.join(rel).to_string_lossy().into_owned();
    }
    let redact: Vec<String> = ctx
        .secrets
        .values()
        .filter_map(Value::as_str)
        .filter(|s| s.len() >= 4)
        .map(str::to_string)
        .chain(ctx.plugin_token.clone())
        .collect();
    let mut cmd = Cmd::new(argv)
        .cwd(&root)
        .stdin(serde_json::to_vec(stdin).expect("json"))
        .timeout(timeout)
        .redact(redact)
        .as_agent()
        .env("XHERD_BIN", &ctx.xherd_bin)
        .env("XHERD_URL", &ctx.xherd_url)
        .env("XHERD_PLUGIN_ID", p.name())
        .env("XHERD_PLUGIN_ROOT", root.to_string_lossy())
        .env("XHERD_PLUGIN_CONFIG_DIR", ctx.config_dir.to_string_lossy())
        .env("XHERD_PLUGIN_STATE_DIR", ctx.state_dir.to_string_lossy())
        .env("XHERD_CONTEXT_JSON", ctx.context.to_string());
    if let Some(t) = &ctx.plugin_token {
        cmd = cmd.env("XHERD_PLUGIN_TOKEN", t);
    }
    cmd
}

/// Run a manifest command: stdin `{command, input, settings, secrets, agent}`,
/// stdout JSON (R-503). stderr goes to the log, redacted.
pub async fn run_command(
    p: &Plugin,
    name: &str,
    input: Value,
    ctx: &Context,
) -> Result<Value, PluginError> {
    if p.builtin() {
        return Err(PluginError::Builtin {
            plugin: p.name().into(),
            name: name.into(),
        });
    }
    let argv = p
        .manifest
        .commands
        .get(name)
        .ok_or_else(|| PluginError::Unknown {
            plugin: p.name().into(),
            what: "command",
            name: name.into(),
        })?;
    let stdin = json!({
        "command": name, "input": input, "settings": ctx.settings,
        "secrets": ctx.secrets, "agent": ctx.agent,
    });
    let secs = p.manifest.timeout_seconds;
    let out = run(build_cmd(p, argv, ctx, &stdin, Duration::from_secs(secs))).await?;
    if !out.stderr.trim().is_empty() {
        tracing::info!(plugin = %p.name(), command = name, stderr = %out.stderr.trim(), "plugin stderr");
    }
    let (plugin, name) = (p.name().to_string(), name.to_string());
    if out.timed_out {
        return Err(PluginError::Timeout { plugin, name, secs });
    }
    if out.code != Some(0) {
        return Err(PluginError::Failed {
            plugin,
            name,
            code: out.code,
            stderr: out.stderr.trim().to_string(),
        });
    }
    serde_json::from_slice(&out.stdout).map_err(|e| PluginError::BadJson {
        plugin,
        name,
        error: e.to_string(),
    })
}

/// [`run_command`] and deserialize the result; a mismatch is a shape failure.
pub async fn run_typed<T: DeserializeOwned>(
    p: &Plugin,
    name: &str,
    input: Value,
    ctx: &Context,
) -> Result<T, PluginError> {
    let v = run_command(p, name, input, ctx).await?;
    serde_json::from_value(v).map_err(|e| PluginError::Shape {
        plugin: p.name().into(),
        name: name.into(),
        error: e.to_string(),
    })
}

/// Run an event hook if the plugin declares one. Output is ignored; failures are logged.
pub async fn run_hook(p: &Plugin, event: &str, payload: Value, ctx: &Context) {
    let Some(argv) = p.manifest.hooks.get(event) else {
        return;
    };
    if p.builtin() {
        return;
    }
    let mut stdin = payload;
    if let Some(o) = stdin.as_object_mut() {
        o.insert("event".into(), event.into());
        o.insert("settings".into(), Value::Object(ctx.settings.clone()));
        o.insert("secrets".into(), Value::Object(ctx.secrets.clone()));
    }
    let cmd = build_cmd(
        p,
        argv,
        ctx,
        &stdin,
        Duration::from_secs(p.manifest.timeout_seconds),
    );
    match run(cmd).await {
        Ok(o) if o.success() => {}
        Ok(o) => {
            tracing::warn!(plugin = %p.name(), event, code = ?o.code, timed_out = o.timed_out, stderr = %o.stderr.trim(), "plugin hook failed")
        }
        Err(e) => tracing::warn!(plugin = %p.name(), event, error = %e, "plugin hook failed"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub plugin: String,
    pub name: String,
    pub ok: bool,
    pub optional: bool,
    pub fixable: bool,
    pub message: String,
}

pub const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// Run a plugin's checks: pass = exit 0 within 10 s (R-502).
pub async fn run_checks(p: &Plugin, ctx: &Context) -> Vec<CheckResult> {
    let mut out = Vec::new();
    for c in &p.manifest.all_checks() {
        let (ok, message) = if p.builtin() && c.argv[0].starts_with("./") {
            (false, "built-in plugin cannot run a ./ check".to_string())
        } else {
            let mut cmd = if p.builtin() {
                Cmd::new(c.argv.clone()).timeout(CHECK_TIMEOUT).as_agent()
            } else {
                build_cmd(p, &c.argv, ctx, &Value::Null, CHECK_TIMEOUT)
            };
            cmd.stdin = None;
            match run(cmd).await {
                Ok(o) if o.success() => (true, String::new()),
                Ok(o) if o.timed_out => (false, "timed out after 10s".into()),
                Ok(o) => (
                    false,
                    o.stderr
                        .lines()
                        .last()
                        .unwrap_or("failed")
                        .trim()
                        .to_string(),
                ),
                Err(e) => (false, e.to_string()),
            }
        };
        out.push(CheckResult {
            plugin: p.name().into(),
            name: c.name.clone(),
            ok,
            optional: c.optional,
            fixable: c.fix.is_some(),
            message,
        });
    }
    out
}
