//! Runtime CLIs install themselves in the container (R3, R5, R6, R7): at
//! start and when a runtime is switched on, each runtime requirement that
//! has an `install` is probed and installed when it is missing or older than
//! its pinned `version`. One install per tool, never retried by this code.

use std::time::Duration;

use xherd_core::exec::{self, Cmd};
use xherd_plugin::manifest::{Requirement, parse_version, version_in};
use xherd_plugin::{Kind, Plugin};

use crate::create::refuse;
use crate::{ActionError, Change, Daemon};

/// An install that takes longer is killed (R3).
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Lines of an install's output kept when it fails.
const TAIL_LINES: usize = 20;

/// What an automatic install is doing; no entry = idle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallState {
    Running {
        /// The pinned version being installed.
        version: Option<String>,
        /// A working older CLI is being replaced (not installed from nothing).
        update: bool,
    },
    /// The last attempt failed; the last lines of its output.
    Failed { tail: String },
}

fn key(plugin: &str, tool: &str) -> String {
    format!("{plugin}:{tool}")
}

/// Whether `out` (a `--version` output) is older than `pin`. A version that
/// cannot be read counts as present: never reinstall in a loop (R6).
fn is_older(out: &str, pin: Option<&str>) -> bool {
    let (Some(pin), Some(have)) = (pin.and_then(parse_version), version_in(out)) else {
        return false;
    };
    parse_version(have).is_some_and(|have| have < pin)
}

fn tail(stdout: &[u8], stderr: &str) -> String {
    let all = format!("{}{}", String::from_utf8_lossy(stdout), stderr);
    let lines: Vec<&str> = all.lines().collect();
    lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n")
}

impl Daemon {
    /// The state of one tool's automatic install.
    pub fn install_state(&self, plugin: &str, tool: &str) -> Option<InstallState> {
        self.installs
            .lock()
            .unwrap()
            .get(&key(plugin, tool))
            .cloned()
    }

    /// `ensure_runtime` for every runtime switched on (end of `start`).
    pub fn ensure_runtimes(&self) {
        let names: Vec<String> = self
            .plugins
            .of_kind(Kind::Runtime)
            .map(|p| p.name().to_string())
            .collect();
        for n in names {
            self.ensure_runtime(&n);
        }
    }

    /// Install what a runtime lacks, in the background. Does nothing unless
    /// `runtime_install` is on and the runtime is switched on (R9).
    pub fn ensure_runtime(&self, plugin: &str) {
        if !self.runtime_install || !self.plugin_enabled(plugin) {
            return;
        }
        let Some(p) = self
            .plugins
            .get(plugin)
            .filter(|p| p.manifest.kind == Kind::Runtime)
        else {
            return;
        };
        for r in p.manifest.requires.iter().filter(|r| r.install.is_some()) {
            let (d, p, r) = (self.clone(), p.clone(), r.clone());
            tokio::spawn(async move { d.auto_install(&p, &r).await });
        }
    }

    /// A job needs the runtime's CLI (R8): refuse while it installs or when
    /// it is missing. Only with `runtime_install` (the container, R9).
    pub(crate) async fn runtime_ready(&self, name: &str) -> Result<(), ActionError> {
        if !self.runtime_install {
            return Ok(());
        }
        let Some(p) = self.plugins.get(name) else {
            return Ok(());
        };
        let ctx = self.plugin_context(p);
        for r in p.manifest.requires.iter().filter(|r| r.install.is_some()) {
            if matches!(
                self.install_state(name, &r.name),
                Some(InstallState::Running { .. })
            ) {
                return Err(refuse(
                    "runtime_missing",
                    format!("{} is being installed: try again in a few minutes", r.name),
                ));
            }
            if let Err(e) = xherd_plugin::probe(p, &ctx, r).await {
                return Err(refuse(
                    "runtime_missing",
                    format!(
                        "{} is not installed ({e}): install it from Settings, Setup checks",
                        r.name
                    ),
                ));
            }
        }
        Ok(())
    }

    async fn auto_install(&self, p: &Plugin, r: &Requirement) {
        let (plugin, tool) = (p.name(), r.name.as_str());
        let ctx = self.plugin_context(p);
        let update = match xherd_plugin::probe(p, &ctx, r).await {
            Ok(out) if !is_older(&out, r.version.as_deref()) => {
                self.installs.lock().unwrap().remove(&key(plugin, tool));
                return;
            }
            Ok(_) => true,
            Err(_) => false,
        };
        {
            let mut installs = self.installs.lock().unwrap();
            let k = key(plugin, tool);
            if matches!(installs.get(&k), Some(InstallState::Running { .. })) {
                return;
            }
            installs.insert(
                k,
                InstallState::Running {
                    version: r.version.clone(),
                    update,
                },
            );
        }
        let _ = self.hub.send(Change::Checks);

        let mut cmd = Cmd::new(r.install.clone().unwrap_or_default())
            .as_agent()
            .timeout(INSTALL_TIMEOUT);
        for (k, v) in r.install_env(std::env::consts::ARCH) {
            cmd = cmd.env(k, v);
        }
        let failure = match exec::run(cmd).await {
            Ok(o) if o.code == Some(0) => None,
            Ok(o) => Some(tail(&o.stdout, &o.stderr)),
            Err(e) => Some(e.to_string()),
        };
        let probed = xherd_plugin::probe(p, &ctx, r).await;
        let state = match (failure, probed) {
            (None, Ok(out)) => {
                let version = version_in(&out).unwrap_or_default();
                tracing::info!(plugin, tool, version, "runtime installed");
                None
            }
            (failure, probed) => {
                let tail = failure.or(probed.err()).unwrap_or_default();
                tracing::warn!(plugin, tool, %tail, "runtime install failed");
                Some(InstallState::Failed { tail })
            }
        };
        {
            let mut installs = self.installs.lock().unwrap();
            match state {
                Some(s) => installs.insert(key(plugin, tool), s),
                None => installs.remove(&key(plugin, tool)),
            };
        }
        let _ = self.hub.send(Change::Checks);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_than_the_pin() {
        assert!(is_older("2.1.318 (Claude Code)", Some("2.1.319")));
        assert!(!is_older("2.1.319 (Claude Code)", Some("2.1.319")));
        assert!(!is_older("codex-cli 0.200.0", Some("0.167.2")));
        assert!(!is_older("odd output", Some("1.0.0")));
        assert!(!is_older("1.0.0", None));
    }

    #[test]
    fn the_tail_keeps_the_last_lines() {
        let out: String = (1..=30).map(|i| format!("l{i}\n")).collect();
        let t = tail(out.as_bytes(), "");
        assert_eq!(t.lines().count(), TAIL_LINES);
        assert!(t.ends_with("l30") && t.starts_with("l11"));
    }
}
