//! Settings → Toolchains (R-653): the tools the toolchain plugins provide, the
//! default version of each (`toolchainDefaults`, used by a worktree without a
//! version file) and installing a version in a terminal the UI shows. Installs
//! happen only in the container; native installs never touch the host.

use std::path::PathBuf;

use serde::Serialize;
use xherd_plugin::toolchain::{Toolchain, subst, valid_version};

use crate::git_access::PLUGIN_LOGIN_SESSION as INSTALL_SESSION;
use crate::{ActionError, Daemon};

/// The install runs in the same tmux session as plugin logins, shown at
/// `/ws/setup/plugin-login`.
pub use crate::git_access::PLUGIN_LOGIN_ID as INSTALL_ID;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub plugin: String,
    pub plugin_label: String,
    pub tool: String,
    /// Saved default version, if any.
    pub default: Option<String>,
    /// The plugin has an install command and xherd runs in the container.
    pub can_install: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    pub version: String,
    pub installed: bool,
}

fn fail(code: &'static str, message: impl Into<String>) -> ActionError {
    ActionError {
        code,
        message: message.into(),
    }
}

impl Daemon {
    /// Every tool of every enabled toolchain plugin (the first plugin claiming
    /// a tool wins, like in a plan).
    pub fn toolchain_tools(&self) -> Vec<ToolInfo> {
        let defaults = self.live_config().toolchain_defaults;
        let mut out: Vec<ToolInfo> = Vec::new();
        for p in self.plugins.of_kind(xherd_plugin::Kind::Toolchain) {
            if !self.plugin_enabled(&p.manifest.name) {
                continue;
            }
            let Ok(tc) = Toolchain::of(&p.manifest) else {
                continue;
            };
            for t in tc.tools {
                if out.iter().any(|o| o.tool == t.name) {
                    continue;
                }
                out.push(ToolInfo {
                    plugin: p.manifest.name.clone(),
                    plugin_label: if p.manifest.label.is_empty() {
                        p.manifest.name.clone()
                    } else {
                        p.manifest.label.clone()
                    },
                    default: defaults.get(&t.name).cloned(),
                    can_install: self.sandbox && t.install.is_some(),
                    tool: t.name,
                });
            }
        }
        out
    }

    /// Install `tool` `version` in a terminal session (`/ws/setup/plugin-login`).
    pub async fn start_toolchain_install(
        &self,
        tool: &str,
        version: &str,
    ) -> Result<(), ActionError> {
        if !self.sandbox {
            return Err(fail(
                "not_in_container",
                "xherd installs toolchains only in its container, never on the host",
            ));
        }
        if !valid_version(version) {
            return Err(fail(
                "bad_version",
                format!("{version:?} is not a valid version"),
            ));
        }
        let (tc, t) = self
            .plugins
            .of_kind(xherd_plugin::Kind::Toolchain)
            .filter(|p| self.plugin_enabled(&p.manifest.name))
            .filter_map(|p| Toolchain::of(&p.manifest).ok())
            .find_map(|tc| {
                let t = tc.tools.iter().find(|t| t.name == tool).cloned()?;
                Some((tc, t))
            })
            .ok_or_else(|| fail("no_tool", format!("no toolchain provides {tool}")))?;
        let argv = t
            .install
            .as_deref()
            .map(|a| subst(a, version))
            .ok_or_else(|| fail("not_fixable", format!("{tool} has no install command")))?;
        let env: Vec<(String, String)> = tc
            .pass_env
            .iter()
            .filter_map(|k| xherd_core::runner::agent_env(k).map(|v| (k.clone(), v)))
            .collect();
        let tmux = xherd_term::Tmux::new(self.config.tmux_server());
        if tmux.has_session(INSTALL_SESSION).await {
            let _ = tmux.kill_session(INSTALL_SESSION).await;
        }
        let home =
            PathBuf::from(xherd_core::runner::agent_env("HOME").unwrap_or_else(|| "/".into()));
        tmux.new_session(INSTALL_SESSION, &home, &env, &argv)
            .await
            .map_err(|e| fail("driver", e.to_string()))
    }

    /// The versions the tool's plugin offers (`list` command, best first),
    /// each marked installed when `resolve` finds it. Container only: the
    /// list needs the network and the agents' toolchain homes.
    pub async fn toolchain_versions(&self, tool: &str) -> Result<Vec<VersionInfo>, ActionError> {
        if !self.sandbox {
            return Err(fail(
                "not_in_container",
                "xherd lists installable versions only in its container",
            ));
        }
        let (tc, t) = self
            .plugins
            .of_kind(xherd_plugin::Kind::Toolchain)
            .filter(|p| self.plugin_enabled(&p.manifest.name))
            .filter_map(|p| Toolchain::of(&p.manifest).ok())
            .find_map(|tc| {
                let t = tc.tools.iter().find(|t| t.name == tool).cloned()?;
                Some((tc, t))
            })
            .ok_or_else(|| fail("no_tool", format!("no toolchain provides {tool}")))?;
        let list = t
            .list
            .ok_or_else(|| fail("no_list", format!("{tool} has no version list")))?;
        let env: Vec<(String, String)> = tc
            .pass_env
            .iter()
            .filter_map(|k| xherd_core::runner::agent_env(k).map(|v| (k.clone(), v)))
            .collect();
        let run = |argv: Vec<String>, secs| {
            let mut cmd = xherd_core::exec::Cmd::new(argv)
                .as_agent()
                .timeout(std::time::Duration::from_secs(secs));
            for (k, v) in &env {
                cmd = cmd.env(k, v);
            }
            xherd_core::exec::run(cmd)
        };
        let out = run(list, 60)
            .await
            .map_err(|e| fail("list_failed", e.to_string()))?;
        if !out.success() {
            return Err(fail(
                "list_failed",
                format!("the version list failed: {}", out.stderr.trim()),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        let versions: Vec<String> = out
            .stdout_str()
            .lines()
            .map(str::trim)
            .filter(|v| valid_version(v) && seen.insert(v.to_string()))
            .take(200)
            .map(str::to_string)
            .collect();
        let mut result = Vec::with_capacity(versions.len());
        for version in versions {
            let installed = match &t.resolve {
                Some(r) => run(subst(r, &version), 10).await.is_ok_and(|o| o.success()),
                None => false,
            };
            result.push(VersionInfo { version, installed });
        }
        Ok(result)
    }
}
