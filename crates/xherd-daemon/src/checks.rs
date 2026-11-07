//! Setup checks (spec §17.2 "Setup checks / Welcome", design §7.1 `/checks`):
//! what must work before agents can run. Required checks that fail block the
//! UI on the Setup screen.

use std::time::Duration;

use serde::Serialize;
use xherd_core::exec::{Cmd, run};
use xherd_plugin::Kind;
use xherd_plugin::runtime::Terminal;

use crate::{ActionError, Daemon};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Fix {
    /// Something the user runs themselves.
    Command { text: String },
    /// `POST /checks/:id/fix` does it.
    Action { label: String },
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
        .timeout(Duration::from_secs(10));
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
        let mut set = tokio::task::JoinSet::new();
        for p in self.plugins.all() {
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
                let runtime = p.manifest.kind == Kind::Runtime
                    && r.plugin == xherd_flow::approve::DEFAULT_RUNTIME;
                let fix = p
                    .manifest
                    .checks
                    .iter()
                    .find(|c| c.name == r.name)
                    .and_then(|c| c.fix.as_ref())
                    .filter(|_| !r.ok)
                    .map(|argv| Fix::Command {
                        text: argv.join(" "),
                    });
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

        if let Some(c) = self.trust_check() {
            out.push(c);
        }
        out.push(self.pack_check(env).await);
        out
    }

    /// The default runtime trusts the worktrees folder (else every agent stops
    /// at the runtime's trust dialog).
    fn trust_check(&self) -> Option<Check> {
        let p = self.plugins.get(xherd_flow::approve::DEFAULT_RUNTIME)?;
        let rt = Terminal::of(&p.manifest).ok()?;
        let t = rt.trust.as_ref()?;
        let root = self.config.workspace_root().join("worktrees");
        let res = crate::create::check_trust(t, &p.manifest.label, &rt.launch.join(" "), &root);
        let d = root.display();
        Some(Check {
            id: "trust".into(),
            group: "runtime",
            label: format!("{} trusts the worktrees folder", p.manifest.label),
            required: true,
            ok: res.is_ok(),
            detail: match &res {
                Ok(()) => d.to_string(),
                Err(e) => e.message.clone(),
            },
            fix: res.is_err().then(|| Fix::Command {
                text: format!("mkdir -p \"{d}\" && cd \"{d}\" && {}", rt.launch.join(" ")),
            }),
        })
    }

    /// The xhSpec commands are where the default runtime finds them.
    async fn pack_check(&self, env: &(dyn Fn(&str) -> Option<String> + Sync)) -> Check {
        let rt = xherd_flow::approve::DEFAULT_RUNTIME;
        let missing = match self.packs.get(xherd_flow::XHSPEC) {
            Some(p) => p
                .plan_install(rt, xherd_flow::install::XHSPEC_FILES, env)
                .map(|plan| plan.iter().filter(|f| !f.path.exists()).count()),
            None => Err("the xhSpec pack is not loaded".into()),
        };
        let (ok, detail) = match missing {
            Ok(0) => (true, "installed".to_string()),
            Ok(n) => (false, format!("{n} command files missing")),
            Err(e) => (false, e),
        };
        Check {
            id: PACK_CHECK.into(),
            group: "runtime",
            label: format!("xhSpec commands installed for {rt}"),
            required: true,
            ok,
            detail,
            fix: (!ok).then(|| Fix::Action {
                label: "Install commands".into(),
            }),
        }
    }

    /// `POST /checks/:id/fix`: only the pack install is fixed by xherd.
    pub async fn fix_check(
        &self,
        id: &str,
        env: &(dyn Fn(&str) -> Option<String> + Sync),
    ) -> Result<(), ActionError> {
        if id != PACK_CHECK {
            return Err(ActionError {
                code: "not_fixable",
                message: format!("check {id} has no automatic fix"),
            });
        }
        self.install(Some(xherd_flow::approve::DEFAULT_RUNTIME), None, env)
            .await
            .map(|_| ())
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
