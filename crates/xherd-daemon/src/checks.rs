//! Setup checks (spec §17.2 "Setup checks / Welcome", design §7.1 `/checks`):
//! what must work before agents can run. Required checks that fail block the
//! UI on the Setup screen.

use std::time::Duration;

use serde::Serialize;
use xherd_core::exec::{Cmd, run};
use xherd_plugin::Kind;

use crate::{ActionError, Daemon};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Fix {
    /// Something the user runs themselves.
    Command { text: String },
    /// `POST /checks/:id/fix` does it.
    Action { label: String },
    /// `POST /checks/:id/fix` starts a terminal session the user finishes in
    /// the browser (`/ws/setup/:id`); the check re-runs when it ends.
    /// `terminal` is the setup session the UI attaches to.
    Session {
        label: String,
        explain: String,
        terminal: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: String,
    pub group: &'static str,
    pub label: String,
    pub required: bool,
    pub ok: bool,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
}

/// Lowest tmux supported (exact `=name` targets, `respawn-window -k` flags).
const TMUX_MIN: (u32, u32) = (3, 3);
pub const PACK_CHECK: &str = "pack";

/// `tmux 3.5a`, `tmux next-3.6` → (3, 5).
pub fn tmux_version(out: &str) -> Option<(u32, u32)> {
    let v = out.trim().rsplit([' ', '-']).next()?;
    let mut it = v.split('.');
    let major = it.next()?.parse().ok()?;
    let minor: String = it
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some((major, minor.parse().ok()?))
}

async fn version(argv: &[&str]) -> Result<String, String> {
    let cmd = Cmd::new(argv.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        .timeout(Duration::from_secs(10))
        .as_agent();
    match run(cmd).await {
        Ok(o) if o.success() => Ok(o.stdout_str().trim().to_string()),
        Ok(o) => Err(o
            .stderr
            .trim()
            .lines()
            .last()
            .unwrap_or("failed")
            .to_string()),
        Err(e) => Err(e.to_string()),
    }
}

impl Daemon {
    /// Every check, in display order. `env` resolves `${VAR}` and `HOME`
    /// (install destinations, trust file).
    pub async fn checks(&self, env: &(dyn Fn(&str) -> Option<String> + Sync)) -> Vec<Check> {
        let mut out = Vec::new();

        let git = version(&["git", "--version"]).await;
        out.push(Check {
            id: "git".into(),
            group: "system",
            label: "git".into(),
            required: true,
            ok: git.is_ok(),
            detail: git.unwrap_or_else(|e| format!("git not found: {e}")),
            fix: None,
        });

        let tmux = version(&["tmux", "-V"]).await;
        let (ok, detail) = match &tmux {
            Ok(v) => match tmux_version(v) {
                Some(have) if have >= TMUX_MIN => (true, v.clone()),
                _ => (
                    false,
                    format!(
                        "{v}: xherd needs tmux {}.{} or newer",
                        TMUX_MIN.0, TMUX_MIN.1
                    ),
                ),
            },
            Err(e) => (false, format!("tmux not found: {e}")),
        };
        out.push(Check {
            id: "tmux".into(),
            group: "system",
            label: "tmux".into(),
            required: true,
            ok,
            detail,
            fix: (!ok).then(|| Fix::Command {
                text: "brew install tmux   # or your package manager".into(),
            }),
        });

        // Plugin checks: required only for the default runtime.
        let default_rt = self.live_config().default_runtime;
        let mut set = tokio::task::JoinSet::new();
        for p in self.plugins.all().filter(|p| self.plugin_enabled(p.name())) {
            let p = p.clone();
            let ctx = self.plugin_context(&p);
            set.spawn(async move {
                let results = xherd_plugin::run_checks(&p, &ctx).await;
                (p, results)
            });
        }
        let mut plugin_checks = Vec::new();
        while let Some(Ok((p, results))) = set.join_next().await {
            for r in results {
                let runtime = p.manifest.kind == Kind::Runtime && r.plugin == default_rt;
                let fix = (!r.ok).then(|| self.plugin_fix(&p, &r.name)).flatten();
                plugin_checks.push(Check {
                    id: format!("plugin:{}:{}", r.plugin, r.name),
                    group: "plugins",
                    label: format!("{} · {}", r.plugin, r.name),
                    required: runtime && !r.optional,
                    ok: r.ok,
                    detail: if r.ok { "passes".into() } else { r.message },
                    fix,
                });
            }
        }
        plugin_checks.sort_by(|a, b| a.id.cmp(&b.id));
        out.extend(plugin_checks);

        if self.sandbox {
            // Stated, never silent (R-1203). With a runner the agents are another
            // user; it is pinged on every run, since it can die after the start.
            out.push(match xherd_core::runner::runner_socket() {
                Some(sock) => {
                    let ok = xherd_core::runner::ping(sock).await;
                    Check {
                        id: "runner".into(),
                        group: "security",
                        label: "Agents run as a separate user".into(),
                        required: false,
                        ok,
                        detail: if ok {
                            "The runner answers; the agents cannot read xherd's config.json or secrets.json."
                                .into()
                        } else {
                            format!(
                                "The runner at {} does not answer: agents cannot start or run commands. \
                                 Restart the container (xherd docker update).",
                                sock.display()
                            )
                        },
                        fix: None,
                    }
                }
                None => Check {
                    id: "single-user".into(),
                    group: "security",
                    label: "Agents are kept from xherd's settings and secrets".into(),
                    required: false,
                    ok: false,
                    detail: "No: the daemon and the agents run as the same user in this container, \
                             so an agent can read xherd's config.json and secrets.json. \
                             Keep plugin secrets you would not hand to an agent out of xherd."
                        .into(),
                    fix: None,
                },
            });
        }
        for rt in self.pack_runtimes() {
            out.push(self.pack_check(&rt, rt == default_rt, env).await);
        }
        out
    }

    /// Runtimes switched on that the built-in pack has commands for.
    pub(crate) fn pack_runtimes(&self) -> Vec<String> {
        let Some(p) = self.packs.get(xherd_flow::BUILTIN) else {
            return Vec::new();
        };
        self.plugins
            .of_kind(Kind::Runtime)
            .map(|r| r.name().to_string())
            .filter(|n| p.install.contains_key(n) && self.plugin_enabled(n))
            .collect()
    }

    /// The xherd commands are where the runtime finds them: required for the
    /// default runtime (id `pack`), stated for the others (`pack:<runtime>`).
    async fn pack_check(
        &self,
        rt: &str,
        default: bool,
        env: &(dyn Fn(&str) -> Option<String> + Sync),
    ) -> Check {
        let missing = match self.packs.get(xherd_flow::BUILTIN) {
            Some(p) => p
                .plan_install(rt, xherd_flow::install::BUILTIN_FILES, env)
                .map(|plan| plan.iter().filter(|f| !f.path.exists()).count()),
            None => Err("the xherd pack is not loaded".into()),
        };
        let (ok, detail) = match missing {
            Ok(0) => (true, "installed".to_string()),
            Ok(n) => (false, format!("{n} command files missing")),
            Err(e) => (false, e),
        };
        Check {
            id: match default {
                true => PACK_CHECK.into(),
                false => format!("{PACK_CHECK}:{rt}"),
            },
            group: "runtime",
            label: format!("xherd commands installed for {rt}"),
            required: default,
            ok,
            detail,
            fix: (!ok).then(|| Fix::Action {
                label: "Install commands".into(),
            }),
        }
    }

    /// What fixes a failed plugin check: a requirement's install in a
    /// terminal (container) or its instructions for this OS (native, R-653),
    /// else the check's own `fix` command.
    fn plugin_fix(&self, p: &xherd_plugin::Plugin, check: &str) -> Option<Fix> {
        if let Some(name) = check.strip_prefix(xherd_plugin::manifest::REQUIRES_PREFIX) {
            let r = p.manifest.requires.iter().find(|r| r.name == name)?;
            if self.sandbox && r.install.is_some() {
                return Some(Fix::Session {
                    label: format!("Install {name}"),
                    explain: format!(
                        "Runs `{}` in a terminal, into the agents' home.",
                        r.install.as_ref()?.join(" ")
                    ),
                    terminal: crate::git_access::PLUGIN_LOGIN_ID.into(),
                });
            }
            return r
                .instructions_for(std::env::consts::OS)
                .map(|t| Fix::Command { text: t.into() });
        }
        p.manifest
            .checks
            .iter()
            .find(|c| c.name == check)
            .and_then(|c| c.fix.as_ref())
            .map(|argv| Fix::Command {
                text: argv.join(" "),
            })
    }

    /// `POST /checks/:id/fix`: install the pack, or a plugin's required tool.
    pub async fn fix_check(
        &self,
        id: &str,
        env: &(dyn Fn(&str) -> Option<String> + Sync),
    ) -> Result<(), ActionError> {
        if let Some((plugin, name)) = id
            .strip_prefix("plugin:")
            .and_then(|r| r.split_once(':'))
            .and_then(|(p, n)| Some((p, n.strip_prefix(xherd_plugin::manifest::REQUIRES_PREFIX)?)))
        {
            return self.start_plugin_install(plugin, name).await;
        }
        let rt = match id.split_once(':') {
            None if id == PACK_CHECK => Some(self.live_config().default_runtime),
            Some((PACK_CHECK, rt)) => Some(rt.to_string()),
            _ => None,
        };
        match rt {
            Some(rt) => self.install(Some(&rt), None, env).await.map(|_| ()),
            None => Err(ActionError {
                code: "not_fixable",
                message: format!("check {id} has no automatic fix"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tmux_version;

    #[test]
    fn tmux_versions() {
        assert_eq!(tmux_version("tmux 3.5a"), Some((3, 5)));
        assert_eq!(tmux_version("tmux 3.3"), Some((3, 3)));
        assert_eq!(tmux_version("tmux next-3.6"), Some((3, 6)));
        assert_eq!(tmux_version("tmux 2.9a\n"), Some((2, 9)));
        assert_eq!(tmux_version("nope"), None);
    }
}
