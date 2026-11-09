//! `xherd security check` (R-1210): a summary of the current exposure, from
//! what this version knows. Informational: a warning is something to look at,
//! not a failure.

use serde::Serialize;
use xherd_core::config::Config;

use crate::Daemon;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub id: &'static str,
    pub ok: bool,
    pub label: &'static str,
    pub detail: String,
}

/// What R-1210 lists that this version cannot report yet.
pub const PENDING: &[&str] = &[
    "autonomy levels in use (autonomy comes later in M4)",
    "budgets not set (budgets are not built yet)",
    "branch protection of the base branch (host command not built yet)",
];

fn f(id: &'static str, ok: bool, label: &'static str, detail: impl Into<String>) -> Finding {
    Finding {
        id,
        ok,
        label,
        detail: detail.into(),
    }
}

fn loopback(bind: &str) -> bool {
    bind == "localhost"
        || bind
            .parse::<std::net::IpAddr>()
            .is_ok_and(|a| a.is_loopback())
}

/// Pure part: everything but the plugin list comes from the config.
pub fn findings(c: &Config, sandbox: bool, external_plugins: &[String]) -> Vec<Finding> {
    let mut out = Vec::new();
    out.push(match (loopback(&c.bind), sandbox) {
        (true, _) => f(
            "network",
            true,
            "Network binding",
            format!("listens on {}:{} only", c.bind, c.port),
        ),
        (false, true) => f(
            "network",
            true,
            "Network binding",
            format!(
                "listens on {} inside the container; compose must publish port {} on the host's 127.0.0.1 (`xherd docker doctor` checks it)",
                c.bind, c.port
            ),
        ),
        (false, false) => f(
            "network",
            false,
            "Network binding",
            format!(
                "listens on {}:{}: reachable from the network; bind 127.0.0.1 unless a firewall or proxy protects it",
                c.bind, c.port
            ),
        ),
    });
    out.push(f(
        "user-split",
        false,
        "Daemon and agents",
        "run as the same user, so an agent can read the daemon's config and secrets (R-1203, not built yet)",
    ));
    out.push(if external_plugins.is_empty() {
        f(
            "plugins",
            true,
            "External plugins",
            "none; only built-in plugins run",
        )
    } else {
        f(
            "plugins",
            false,
            "External plugins",
            format!(
                "{} run with your rights and are not pinned to a version: {}",
                external_plugins.len(),
                external_plugins.join(", ")
            ),
        )
    });
    out.push(if c.max_session_minutes > 0 {
        f(
            "session-limit",
            true,
            "Session length",
            format!("a session over {} min is paused", c.max_session_minutes),
        )
    } else {
        f(
            "session-limit",
            false,
            "Session length",
            "no limit: set maxSessionMinutes to pause long sessions",
        )
    });
    out.push(if c.sensitive_paths.is_empty() {
        f(
            "sensitive-paths",
            false,
            "Sensitive paths",
            "none set: set sensitivePaths to hold commits that touch them",
        )
    } else {
        f(
            "sensitive-paths",
            true,
            "Sensitive paths",
            format!("{} glob(s) hold a commit", c.sensitive_paths.len()),
        )
    });
    out
}

impl Daemon {
    pub fn security_check(&self) -> Vec<Finding> {
        let external: Vec<String> = self
            .plugins
            .all()
            .filter(|p| !p.builtin())
            .map(|p| p.name().to_string())
            .collect();
        findings(&self.config, self.sandbox, &external)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(v: &'a [Finding], id: &str) -> &'a Finding {
        v.iter().find(|f| f.id == id).unwrap()
    }

    #[test]
    fn defaults_warn_about_what_is_unset() {
        let v = findings(&Config::default(), false, &[]);
        assert!(get(&v, "network").ok);
        assert!(get(&v, "plugins").ok);
        assert!(!get(&v, "session-limit").ok);
        assert!(!get(&v, "sensitive-paths").ok);
        assert!(!get(&v, "user-split").ok);
    }

    #[test]
    fn open_bind_warns_outside_the_container_only() {
        let c = Config {
            bind: "0.0.0.0".into(),
            ..Default::default()
        };
        assert!(!get(&findings(&c, false, &[]), "network").ok);
        assert!(get(&findings(&c, true, &[]), "network").ok);
    }

    #[test]
    fn set_limits_and_external_plugins() {
        let c = Config {
            max_session_minutes: 60,
            sensitive_paths: vec!["infra/**".into()],
            ..Default::default()
        };
        let v = findings(&c, false, &["mine".into()]);
        assert!(get(&v, "session-limit").ok && get(&v, "sensitive-paths").ok);
        let p = get(&v, "plugins");
        assert!(!p.ok && p.detail.contains("mine"));
    }
}
