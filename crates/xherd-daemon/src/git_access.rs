//! Git access per host (spec §12.3, design §8.4): the git config xherd
//! manages (credential helpers, CA bundles, optional URL rewrites), answers
//! for `xherd credential`, the doctor (R-687), accepting a host key (R-683)
//! and the browser login session (R-686).
//!
//! Hosts and tokens are read from `config.json` / `secrets.json` on every
//! call, so Settings → Git access applies without a restart.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use xherd_core::config::{Config, Credential, GIT_SECRETS, GitHost, GitIdentity, Secrets};
use xherd_core::exec::{Cmd, run};
use xherd_core::paths::write_atomic;
use xherd_plugin::Kind;
use xherd_plugin::git::{Git, subst_host};

use crate::checks::Fix;
use crate::{ActionError, Daemon};

/// tmux session of the browser login (`gh` / `glab` device flow).
pub const LOGIN_SESSION: &str = "xherd-setup-git-login";
pub const LOGIN_ID: &str = "git-login";
pub const PLUGIN_LOGIN_SESSION: &str = "xherd-setup-plugin-login";
pub const PLUGIN_LOGIN_ID: &str = "plugin-login";

fn fail(code: &'static str, message: impl Into<String>) -> ActionError {
    ActionError {
        code,
        message: message.into(),
    }
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn user_section(name: &str, email: &str) -> String {
    let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "[user]\n\tname = \"{}\"\n\temail = \"{}\"\n",
        q(name),
        q(email)
    )
}

/// Folder next to the managed gitconfig that holds one file per identity rule
/// (`includeIf` paths are relative to the including file).
pub const IDENTITIES_DIR: &str = "identities";

fn identity_file_name(n: usize) -> String {
    format!("{IDENTITIES_DIR}/{n}.gitconfig")
}

/// The managed git config (`~/.config/xherd/gitconfig`, included from
/// `~/.gitconfig` in the container). Pure, so it is tested directly.
/// `helper_for` gives the credential helper of a host's provider (D24: it is
/// in the provider's manifest).
pub fn managed_gitconfig(
    hosts: &[GitHost],
    xherd_bin: &str,
    sandbox: bool,
    identity: Option<(&str, &str)>,
    rules: &[GitIdentity],
    helper_for: &dyn Fn(&GitHost) -> Option<String>,
) -> String {
    let mut out =
        String::from("# Managed by xherd (Settings → Git access); rewritten on every change.\n");
    if sandbox {
        // One user owns every file in the container (design §8.4).
        out.push_str("[safe]\n\tdirectory = *\n");
    }
    if let Some((name, email)) = identity {
        out.push_str(&user_section(name, email));
    }
    // After the global identity, so a matching rule wins. The patterns cover
    // HTTPS, scp-like SSH and ssh:// remotes (git 2.36 or newer).
    for (n, r) in rules.iter().enumerate() {
        let (host, owner) = match r.scope.split_once('/') {
            Some((h, o)) => (h, format!("{o}/")),
            None => (r.scope.as_str(), String::new()),
        };
        let bare_host = host.split(':').next().unwrap_or(host);
        for pattern in [
            format!("https://{}/**", r.scope),
            format!("git@{bare_host}:{owner}**"),
            format!("ssh://git@{}/**", r.scope),
        ] {
            out.push_str(&format!(
                "[includeIf \"hasconfig:remote.*.url:{pattern}\"]\n\tpath = {}\n",
                identity_file_name(n)
            ));
        }
    }
    for h in hosts {
        let helper = match h.credential {
            Credential::None => None,
            Credential::Xherd => Some(format!("!{} credential", quote(xherd_bin))),
            // gh reads the host from git on stdin; it has no --hostname flag.
            Credential::Provider => helper_for(h),
        };
        if helper.is_some() || h.username.is_some() {
            out.push_str(&format!("[credential \"https://{}\"]\n", h.host));
            if let Some(helper) = helper {
                // An empty value first resets helpers inherited from elsewhere.
                out.push_str(&format!("\thelper =\n\thelper = {helper}\n"));
            }
            if let Some(u) = &h.username {
                out.push_str(&format!("\tusername = {u}\n"));
            }
        }
        if let Some(ca) = &h.ca_file {
            out.push_str(&format!(
                "[http \"https://{}/\"]\n\tsslCAInfo = {ca}\n",
                h.host
            ));
        }
        if h.prefer_https {
            out.push_str(&format!(
                "[url \"https://{0}/\"]\n\tinsteadOf = git@{0}:\n\tinsteadOf = ssh://git@{0}/\n",
                h.host
            ));
        }
        if h.prefer_ssh {
            out.push_str(&format!(
                "[url \"git@{0}:\"]\n\tinsteadOf = https://{0}/\n",
                h.host
            ));
        }
    }
    out
}

/// The username git sends with a token for a host no provider claims.
const DEFAULT_TOKEN_USER: &str = "x-access-token";

/// A provider: its plugin name and its `data.git`.
type Provider = (String, Git);

/// The provider a host is bound to: the one named, else the first claiming
/// the host name (so a host added before providers existed keeps its API
/// checks and token user).
fn bound<'a>(providers: &'a [Provider], named: Option<&str>, host: &str) -> Option<&'a Git> {
    match named {
        Some(n) => providers.iter().find(|(p, _)| p == n),
        None => providers.iter().find(|(_, g)| g.claims(host)),
    }
    .map(|(_, g)| g)
}

/// The user name for a host's token: its provider's, else [`DEFAULT_TOKEN_USER`].
fn token_user_of(providers: &[Provider], named: Option<&str>, host: &str) -> String {
    bound(providers, named, host)
        .and_then(|g| g.credential.as_ref())
        .and_then(|c| c.token_user.clone())
        .unwrap_or_else(|| DEFAULT_TOKEN_USER.to_string())
}

/// `owner/repo` and host of a remote URL (`https://h/o/r(.git)`,
/// `git@h:o/r(.git)`, `ssh://git@h/o/r`).
pub fn parse_remote(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = url.split_once("://").map(|(_, r)| r) {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
        rest.split_once('/')?
    } else {
        let rest = url.rsplit_once('@').map_or(url, |(_, r)| r);
        rest.split_once(':')?
    };
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    (!host.is_empty() && path.contains('/')).then(|| (host.to_string(), path.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: &'static str,
    pub label: String,
    pub ok: bool,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostsUpdate {
    pub hosts: Vec<GitHost>,
    /// Host → new token; `null` removes it; absent keeps it.
    #[serde(default)]
    pub tokens: Map<String, Value>,
}

/// One command's outcome, never with secrets in it.
struct Ran {
    code: Option<i32>,
    out: String,
    err: String,
}

async fn sh(cmd: Cmd) -> Ran {
    match run(cmd.timeout(Duration::from_secs(20))).await {
        Ok(o) => Ran {
            code: if o.timed_out { None } else { o.code },
            out: o.stdout_str(),
            err: o.stderr,
        },
        Err(e) => Ran {
            code: None,
            out: String::new(),
            err: e.to_string(),
        },
    }
}

fn last_line(s: &str) -> String {
    s.trim().lines().last().unwrap_or("").trim().to_string()
}

fn cmd_fix(text: impl Into<String>) -> Option<Fix> {
    Some(Fix::Command { text: text.into() })
}

/// The host of a remote URL (`https://h/..`, `ssh://git@h:22/..`,
/// `git@h:o/r`); lower-case, no user or port.
pub fn remote_host(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://").map_or(url.trim(), |(_, r)| r);
    let authority = rest.split('/').next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = authority.split(':').next()?.to_ascii_lowercase();
    (!host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
    .then_some(host)
}

impl Daemon {
    /// `data.git` of every enabled host plugin that has one (a broken one is
    /// logged and skipped).
    fn providers(&self) -> Vec<Provider> {
        self.plugins
            .of_kind(Kind::Host)
            .filter(|p| self.plugin_enabled(p.name()))
            .filter_map(|p| match Git::of(&p.manifest) {
                Ok(g) => g.map(|g| (p.name().to_string(), g)),
                Err(e) => {
                    tracing::warn!(plugin = p.name(), "{e}");
                    None
                }
            })
            .collect()
    }

    /// The git data of the provider a host is bound to.
    fn provider_of(&self, h: &GitHost) -> Option<Git> {
        bound(&self.providers(), h.provider.as_deref(), &h.host).cloned()
    }

    /// The user name for a host's token.
    fn token_user(&self, host: &str) -> String {
        let named = self
            .live_config()
            .git_hosts
            .into_iter()
            .find(|h| h.host == host)
            .and_then(|h| h.provider);
        token_user_of(&self.providers(), named.as_deref(), host)
    }

    /// R-688: the doctor's answer for the host an agent works against (its
    /// PR's, else its worktree's `origin`), for the blocked band.
    pub async fn agent_git_access(&self, who: &str) -> Result<Value, ActionError> {
        let a = self.action(who, crate::actor::UserAction::Get).await?;
        let origin = match a.workdir.as_deref() {
            Some(wd) => {
                let r = sh(
                    Cmd::new(["git", "-C", wd, "config", "--get", "remote.origin.url"]).as_agent(),
                )
                .await;
                (r.code == Some(0)).then(|| r.out.trim().to_string())
            }
            None => None,
        };
        let host = a
            .pr_url
            .as_deref()
            .and_then(remote_host)
            .or_else(|| origin.as_deref().and_then(remote_host))
            .ok_or_else(|| ActionError {
                code: "no_host",
                message: "this agent has no remote to check".into(),
            })?;
        let findings = self.git_doctor(&host, origin.as_deref()).await;
        Ok(serde_json::json!({"host": host, "findings": findings}))
    }

    pub(crate) fn live_config(&self) -> Config {
        Config::load(&self.paths.config())
            .map(|(c, _)| c)
            .unwrap_or_else(|_| (*self.config).clone())
    }

    /// Plugins can be switched off while xherd runs; the saved config decides.
    pub fn plugin_enabled(&self, name: &str) -> bool {
        !self
            .live_config()
            .disabled_plugins
            .iter()
            .any(|n| n == name)
    }

    fn live_secrets(&self) -> Secrets {
        Secrets::load(&self.paths.secrets()).unwrap_or_else(|_| (*self.secrets).clone())
    }

    /// The managed git config: in the container `~/.config/xherd/gitconfig`
    /// (included from `~/.gitconfig` by `xherd init`); natively a file in the
    /// data root that the user may include (xherd never edits their config).
    pub fn gitconfig_path(&self) -> Option<PathBuf> {
        if self.sandbox {
            // A folder the daemon owns and the agents' user can read: they
            // include the file but cannot change it (R-1203).
            let dir = std::env::var_os("XHERD_SHARED")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(xherd_core::container::SHARED));
            Some(dir.join("gitconfig"))
        } else {
            Some(self.paths.root.join("gitconfig"))
        }
    }

    /// Rewrite the managed git config (at start and on every change).
    pub fn write_gitconfig(&self) -> Result<(), ActionError> {
        let Some(p) = self.gitconfig_path() else {
            return Ok(());
        };
        let c = self.live_config();
        let identity = match (&c.git_user_name, &c.git_user_email) {
            (Some(n), Some(e)) => Some((n.as_str(), e.as_str())),
            _ => None,
        };
        let text = managed_gitconfig(
            &c.git_hosts,
            &self.xherd_bin,
            self.sandbox,
            identity,
            &c.git_identities,
            &|h| {
                self.provider_of(h)
                    .and_then(|g| g.credential)
                    .and_then(|c| c.helper)
            },
        );
        // The rule files first, so the config never points at a missing one.
        let dir = p.parent().unwrap_or(Path::new(".")).join(IDENTITIES_DIR);
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        for (n, r) in c.git_identities.iter().enumerate() {
            let f = dir.join(format!("{n}.gitconfig"));
            write_atomic(&f, user_section(&r.name, &r.email).as_bytes(), Some(0o644))
                .map_err(|e| fail("io", format!("{}: {e}", f.display())))?;
        }
        write_atomic(&p, text.as_bytes(), Some(0o644))
            .map_err(|e| fail("io", format!("{}: {e}", p.display())))
    }

    /// `GET /git/hosts`: hosts with whether a token is stored (never the token).
    pub fn git_hosts(&self) -> Value {
        let s = self.live_secrets();
        let hosts: Vec<Value> = self
            .live_config()
            .git_hosts
            .iter()
            .map(|h| {
                let mut v = serde_json::to_value(h).expect("json");
                v["hasToken"] = s.git_token(&h.host).is_some().into();
                v
            })
            .collect();
        let providers: Vec<Value> = self
            .providers()
            .into_iter()
            .map(|(name, g)| {
                serde_json::json!({
                    "name": name,
                    "hosts": g.hosts,
                    "helper": g.credential.as_ref().is_some_and(|c| c.helper.is_some()),
                    "login": g.login.is_some(),
                })
            })
            .collect();
        serde_json::json!({
            "hosts": hosts,
            "providers": providers,
            "sandbox": self.sandbox,
            "gitconfig": self.gitconfig_path(),
        })
    }

    /// `PUT /git/hosts`: save hosts and tokens, rewrite the managed config.
    pub fn set_git_hosts(&self, u: HostsUpdate) -> Result<Value, ActionError> {
        let mut seen = std::collections::BTreeSet::new();
        let providers = self.providers();
        for h in &u.hosts {
            let ok = !h.host.is_empty()
                && h.host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
            if !ok {
                return Err(fail(
                    "bad_config",
                    format!("{:?} is not a host name", h.host),
                ));
            }
            if !seen.insert(h.host.clone()) {
                return Err(fail("bad_config", format!("{} is listed twice", h.host)));
            }
            if h.prefer_https && h.prefer_ssh {
                return Err(fail(
                    "bad_config",
                    format!("{}: prefer HTTPS or SSH, not both", h.host),
                ));
            }
            if h.protocols.iter().any(|p| p != "https" && p != "ssh") {
                return Err(fail("bad_config", "protocols are https and ssh"));
            }
            if let Some(p) = &h.provider
                && !providers.iter().any(|(n, _)| n == p)
            {
                return Err(fail(
                    "bad_config",
                    format!("{}: {p:?} is not a git provider plugin", h.host),
                ));
            }
        }
        let mut c = self.live_config();
        c.git_hosts = u.hosts;
        c.save(&self.paths.config())
            .map_err(|e| fail("io", e.to_string()))?;
        if !u.tokens.is_empty() {
            let mut s = self.live_secrets();
            let mut git = s.for_plugin(GIT_SECRETS);
            for (host, t) in u.tokens {
                match t.as_str().map(str::trim) {
                    Some(t) if !t.is_empty() => {
                        git.insert(host, t.into());
                    }
                    _ => {
                        git.remove(&host);
                    }
                }
            }
            s.0.insert(GIT_SECRETS.into(), Value::Object(git));
            s.save(&self.paths.secrets())
                .map_err(|e| fail("io", e.to_string()))?;
        }
        tracing::info!("git access settings changed");
        // The actors pick a pull-request host by these rows (D24).
        self.settings.set(crate::actor::ActorSettings::from_config(
            &c,
            (*self.settings.get()).clone(),
            self.pr_poll_override,
        ));
        self.write_gitconfig()?;
        Ok(self.git_hosts())
    }

    /// `xherd credential get` (R-684): the stored token for a configured host
    /// whose credential source is xherd. Every answer is logged.
    pub fn credential(&self, protocol: &str, host: &str) -> Option<(String, String)> {
        if protocol != "https" {
            return None;
        }
        let c = self.live_config();
        let h = c
            .git_hosts
            .iter()
            .find(|h| h.host == host && h.credential == Credential::Xherd)?;
        let token = self.live_secrets().git_token(host)?;
        tracing::info!(host, "git credential handed out");
        Some((
            h.username.clone().unwrap_or_else(|| self.token_user(host)),
            token,
        ))
    }

    /// Fingerprints `ssh-keyscan` sees for a host (R-683).
    async fn scan_host(host: &str) -> Vec<(String, String)> {
        let (name, port) = match host.rsplit_once(':') {
            Some((n, p)) if p.chars().all(|c| c.is_ascii_digit()) => (n, Some(p)),
            _ => (host, None),
        };
        let mut argv = vec!["ssh-keyscan", "-T", "10"];
        if let Some(p) = port {
            argv.extend(["-p", p]);
        }
        argv.push(name);
        let scan = sh(Cmd::new(argv.iter().copied())).await;
        let mut out = Vec::new();
        for line in scan
            .out
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        {
            let fp = sh(Cmd::new(["ssh-keygen", "-lf", "-"]).stdin(format!("{line}\n"))).await;
            if let Some(f) = fp.out.split_whitespace().nth(1) {
                out.push((f.to_string(), line.to_string()));
            }
        }
        out
    }

    /// Add a host key the user accepted, if `ssh-keyscan` still sees exactly
    /// that fingerprint (container: xherd's known_hosts; native: the user's).
    pub async fn accept_host_key(&self, host: &str, fingerprint: &str) -> Result<(), ActionError> {
        // The agents' user's home: its ssh config includes this file (R-1203).
        let home =
            xherd_core::runner::agent_env("HOME").ok_or_else(|| fail("io", "HOME is not set"))?;
        let file = if self.sandbox {
            Path::new(&home).join(".config/xherd/known_hosts")
        } else {
            Path::new(&home).join(".ssh/known_hosts")
        };
        let lines: Vec<String> = Self::scan_host(host)
            .await
            .into_iter()
            .filter(|(f, _)| f == fingerprint)
            .map(|(_, l)| l)
            .collect();
        if lines.is_empty() {
            return Err(fail(
                "fingerprint_changed",
                format!("{host} no longer shows {fingerprint}; run the doctor again"),
            ));
        }
        let mut text = xherd_core::runner::read_file(&file)
            .await
            .ok()
            .flatten()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        for l in lines {
            text.push_str(&l);
            text.push('\n');
        }
        xherd_core::runner::write_file(&file, text.as_bytes(), Some(0o644))
            .await
            .map_err(|e| fail("io", format!("{}: {e}", file.display())))?;
        tracing::info!(host, fingerprint, "host key accepted");
        Ok(())
    }

    /// Start `gh auth login --web` (or glab) in a tmux session the UI shows
    /// (`/ws/setup/git-login`); the code to type appears there (R-686).
    pub async fn start_git_login(&self, host: &str) -> Result<(), ActionError> {
        let configured = self
            .live_config()
            .git_hosts
            .into_iter()
            .find(|h| h.host == host);
        let login = configured
            .as_ref()
            .and_then(|h| self.provider_of(h))
            .or_else(|| bound(&self.providers(), None, host).cloned())
            .and_then(|g| g.login)
            .ok_or_else(|| {
                fail(
                    "not_fixable",
                    format!("{host}'s provider has no browser login; paste a token instead"),
                )
            })?;
        let argv = subst_host(&login, host);
        let tmux = xherd_term::Tmux::new(self.config.tmux_server());
        if tmux.has_session(LOGIN_SESSION).await {
            let _ = tmux.kill_session(LOGIN_SESSION).await;
        }
        let home =
            PathBuf::from(xherd_core::runner::agent_env("HOME").unwrap_or_else(|| "/".into()));
        tmux.new_session(LOGIN_SESSION, &home, &[], &argv)
            .await
            .map_err(|e| fail("driver", e.to_string()))
    }

    /// Run a plugin's `login` argv in a tmux session the UI shows
    /// (`/ws/setup/plugin-login`).
    pub async fn start_plugin_login(&self, plugin: &str) -> Result<(), ActionError> {
        let argv = self
            .plugins
            .get(plugin)
            .ok_or_else(|| fail("no_plugin", format!("no plugin named {plugin}")))?
            .manifest
            .login
            .clone()
            .ok_or_else(|| fail("not_fixable", format!("{plugin} has no login")))?;
        let tmux = xherd_term::Tmux::new(self.config.tmux_server());
        if tmux.has_session(PLUGIN_LOGIN_SESSION).await {
            let _ = tmux.kill_session(PLUGIN_LOGIN_SESSION).await;
        }
        let home =
            PathBuf::from(xherd_core::runner::agent_env("HOME").unwrap_or_else(|| "/".into()));
        tmux.new_session(PLUGIN_LOGIN_SESSION, &home, &[], &argv)
            .await
            .map_err(|e| fail("driver", e.to_string()))
    }

    /// Run the `install` argv of a plugin's required tool in the login
    /// terminal (`/ws/setup/plugin-login`), container only: a native run
    /// never installs on the host (R-653).
    pub async fn start_plugin_install(&self, plugin: &str, tool: &str) -> Result<(), ActionError> {
        if !self.sandbox {
            return Err(fail(
                "not_fixable",
                "xherd runs natively and does not install tools on this computer; follow the instructions shown",
            ));
        }
        let argv = self
            .plugins
            .get(plugin)
            .ok_or_else(|| fail("no_plugin", format!("no plugin named {plugin}")))?
            .manifest
            .requires
            .iter()
            .find(|r| r.name == tool)
            .and_then(|r| r.install.clone())
            .ok_or_else(|| fail("not_fixable", format!("{plugin} has no install for {tool}")))?;
        let tmux = xherd_term::Tmux::new(self.config.tmux_server());
        if tmux.has_session(PLUGIN_LOGIN_SESSION).await {
            let _ = tmux.kill_session(PLUGIN_LOGIN_SESSION).await;
        }
        let home =
            PathBuf::from(xherd_core::runner::agent_env("HOME").unwrap_or_else(|| "/".into()));
        // Wait for Enter on failure so the error stays on the screen (as for toolchains).
        let script = "\"$@\" || { echo; echo 'Install failed. Press Enter to close.'; read _; }";
        let mut wrapped = vec!["sh".to_string(), "-c".into(), script.into(), "_".into()];
        wrapped.extend(argv);
        tmux.new_session(PLUGIN_LOGIN_SESSION, &home, &[], &wrapped)
            .await
            .map_err(|e| fail("driver", e.to_string()))
    }

    /// `xherd git doctor` (R-687): in order, per protocol the host uses, up to
    /// the first failure; with a repository, its API and git transport too.
    /// Secrets never appear in the findings.
    pub async fn git_doctor(&self, host: &str, repo: Option<&str>) -> Vec<Finding> {
        let c = self.live_config();
        let cfg = c.git_hosts.iter().find(|h| h.host == host).cloned();
        let protocols = cfg
            .as_ref()
            .map(|h| h.protocols.clone())
            .unwrap_or_else(|| vec!["https".into(), "ssh".into()]);
        let mut out = Vec::new();
        if protocols.iter().any(|p| p == "ssh") {
            self.doctor_ssh(host, &mut out).await;
        }
        let ssh_len = out.len();
        if protocols.iter().any(|p| p == "https") {
            self.doctor_https(host, repo, &mut out).await;
        }
        // Both protocols are enabled by default: when HTTPS works, a failing
        // SSH step is not a problem (issue #43), so it is reported as passing.
        if ssh_len > 0 && out.len() > ssh_len && out[ssh_len..].iter().all(|f| f.ok) {
            for f in &mut out[..ssh_len] {
                if !f.ok {
                    f.ok = true;
                    f.detail = format!("not needed, HTTPS works ({})", f.detail);
                    f.fix = None;
                }
            }
        }
        if let Some(url) = repo {
            let r = sh(Cmd::new(["git", "ls-remote", "--heads", url])
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GCM_INTERACTIVE", "never")
                .env(
                    "GIT_SSH_COMMAND",
                    "ssh -o BatchMode=yes -o ConnectTimeout=10",
                )
                .as_agent())
            .await;
            out.push(Finding {
                id: "git-transport",
                label: format!("git can read {url}"),
                ok: r.code == Some(0),
                detail: if r.code == Some(0) {
                    "git ls-remote works".into()
                } else {
                    last_line(&r.err)
                },
                fix: None,
            });
        }
        out
    }

    async fn doctor_ssh(&self, host: &str, out: &mut Vec<Finding>) {
        let sock = std::env::var("SSH_AUTH_SOCK").unwrap_or_default();
        let ok = !sock.is_empty() && Path::new(&sock).exists();
        out.push(Finding {
            id: "ssh-agent",
            label: "SSH agent reachable".into(),
            ok,
            detail: if ok { sock.clone() } else { "SSH_AUTH_SOCK is not set or missing".into() },
            fix: (!ok).then(|| Fix::Command {
                text: if self.sandbox {
                    "forward your SSH agent: `xherd docker init --force` on the host, then recreate the container".into()
                } else {
                    "start an ssh-agent (eval \"$(ssh-agent)\") before the daemon".into()
                },
            }),
        });
        if !ok {
            return;
        }
        let keys = sh(Cmd::new(["ssh-add", "-l"]).as_agent()).await;
        let ok = keys.code == Some(0);
        out.push(Finding {
            id: "ssh-key",
            label: "A key is loaded in the agent".into(),
            ok,
            detail: if ok {
                format!("{} key(s)", keys.out.lines().count())
            } else {
                last_line(&format!("{}{}", keys.out, keys.err))
            },
            fix: (!ok).then(|| Fix::Command {
                text: "ssh-add   # on the host, then try again".into(),
            }),
        });
        if !ok {
            return;
        }
        let known = sh(Cmd::new(["ssh", "-G", host]).as_agent()).await;
        let files: Vec<String> = known
            .out
            .lines()
            .filter_map(|l| l.strip_prefix("userknownhostsfile "))
            .flat_map(|l| l.split_whitespace().map(str::to_string))
            .collect();
        let name = host.split(':').next().unwrap_or(host);
        let mut is_known = false;
        for f in &files {
            // `~` is the agent user's home, which is not ours (R-1203).
            let script = "f=$1; case \"$f\" in \"~\"*) f=\"$HOME${f#\\~}\";; esac; \
                          exec ssh-keygen -F \"$0\" -f \"$f\"";
            if sh(Cmd::new(["sh", "-c", script, name, f]).as_agent())
                .await
                .code
                == Some(0)
            {
                is_known = true;
                break;
            }
        }
        if !is_known {
            let seen = Self::scan_host(host).await;
            let detail = if seen.is_empty() {
                format!("{host} is not in known_hosts and did not answer ssh-keyscan")
            } else {
                format!(
                    "{host} is not in known_hosts. It shows {}; compare with what the host publishes before accepting.",
                    seen.iter()
                        .map(|(f, _)| f.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            out.push(Finding {
                id: "ssh-host-key",
                label: format!("{host}'s host key is known"),
                ok: false,
                detail,
                fix: seen
                    .into_iter()
                    .find(|(f, _)| !f.is_empty())
                    .map(|(f, _)| Fix::Action {
                        label: format!("Accept {f}"),
                    }),
            });
            return;
        }
        out.push(Finding {
            id: "ssh-host-key",
            label: format!("{host}'s host key is known"),
            ok: true,
            detail: "in known_hosts".into(),
            fix: None,
        });
        let r = sh(Cmd::new([
            "ssh",
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            &format!("git@{name}"),
        ])
        .as_agent())
        .await;
        let text = format!("{}{}", r.out, r.err);
        let lower = text.to_lowercase();
        let ok = lower.contains("successfully authenticated")
            || lower.contains("welcome to gitlab")
            || lower.contains("authenticated via")
            || lower.contains("logged in as");
        out.push(Finding {
            id: "ssh-auth",
            label: format!("SSH login to {host}"),
            ok,
            detail: last_line(&text),
            fix: (!ok).then(|| Fix::Command {
                text: format!("add your public key to your {host} account (ssh-add -L shows it)"),
            }),
        });
    }

    async fn doctor_https(&self, host: &str, repo: Option<&str>, out: &mut Vec<Finding>) {
        let url = format!("https://{host}/");
        let helper =
            sh(Cmd::new(["git", "config", "--get-urlmatch", "credential.helper", &url]).as_agent())
                .await;
        let helper = helper.out.trim().to_string();
        let ok = !helper.is_empty();
        out.push(Finding {
            id: "https-helper",
            label: "A git credential helper is set for HTTPS".into(),
            ok,
            detail: if ok { helper.clone() } else { "none".into() },
            fix: (!ok).then(|| Fix::Command {
                text: if self.sandbox {
                    format!("Settings → Git access: choose how {host} gets its token")
                } else {
                    format!(
                        "Settings → Git access, then add `[include] path = {}` to ~/.gitconfig",
                        self.gitconfig_path()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default()
                    )
                },
            }),
        });
        if !ok {
            return;
        }
        let fill = sh(Cmd::new(["git", "credential", "fill"])
            .stdin(format!("protocol=https\nhost={host}\n\n"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .as_agent())
        .await;
        let token = fill
            .out
            .lines()
            .find_map(|l| l.strip_prefix("password="))
            .map(str::to_string)
            .filter(|t| !t.is_empty());
        out.push(Finding {
            id: "https-token",
            label: format!("The helper gives a token for {host}"),
            ok: token.is_some(),
            detail: if token.is_some() {
                "yes".into()
            } else {
                last_line(&fill.err)
            },
            fix: token.is_none().then(|| Fix::Action {
                label: "Log in".into(),
            }),
        });
        let Some(token) = token else { return };
        let named = self
            .live_config()
            .git_hosts
            .into_iter()
            .find(|h| h.host == host)
            .and_then(|h| h.provider);
        let Some(api) =
            bound(&self.providers(), named.as_deref(), host).and_then(|g| g.api.clone())
        else {
            return;
        };
        let base = api.base.replace("{host}", host);
        let get = |path: &str| {
            // The token goes on stdin (`-H @-`), never in argv.
            Cmd::new([
                "curl",
                "-sS",
                "-o",
                "/dev/null",
                "-D",
                "-",
                "-H",
                "@-",
                &format!("{base}{path}"),
            ])
            .stdin(format!("Authorization: Bearer {token}\n"))
            .redact(vec![token.clone()])
        };
        let header = |out: &str, name: &str| {
            let prefix = format!("{}:", name.to_lowercase());
            out.lines().find_map(|l| {
                l.to_lowercase()
                    .strip_prefix(&prefix)
                    .map(|s| s.trim().to_string())
            })
        };
        let me = sh(get(&api.user.path)).await;
        let status = me
            .out
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .to_string();
        let scopes = api
            .user
            .scopes_header
            .as_deref()
            .and_then(|h| header(&me.out, h));
        let mut ok = status == "200";
        let mut detail = if ok {
            "token accepted".to_string()
        } else {
            format!("HTTP {status} {}", last_line(&me.err))
        };
        if ok
            && let (Some(need), Some(s)) = (&api.user.require_scope, &scopes)
            && !s.split(',').any(|x| x.trim() == need)
        {
            ok = false;
            detail = format!("the token has scopes [{s}] but needs {need}");
        }
        out.push(Finding {
            id: "api-token",
            label: format!("{host}'s API accepts the token"),
            ok,
            detail,
            fix: (!ok).then(|| Fix::Action {
                label: "Log in again".into(),
            }),
        });
        let Some(repo_api) = api.repo.as_ref().filter(|_| ok) else {
            return;
        };
        let Some((_, path)) = repo.and_then(parse_remote) else {
            return;
        };
        let r = sh(get(&repo_api.path.replace("{path}", &path))).await;
        let status = r
            .out
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .to_string();
        // The SSO header keeps its original case: it carries a URL.
        let sso = repo_api.sso_header.as_deref().and_then(|h| {
            let prefix = format!("{}:", h.to_lowercase());
            r.out.lines().find_map(|l| {
                l.to_lowercase()
                    .starts_with(&prefix)
                    .then(|| l.split_once(':').map_or("", |x| x.1).trim().to_string())
            })
        });
        let verdict = |v: &xherd_plugin::git::Verdict, url: &str| {
            (
                false,
                v.detail.clone(),
                v.fix
                    .as_ref()
                    .and_then(|f| cmd_fix(f.replace("{url}", url))),
            )
        };
        let (ok, detail, fix) = match (status.as_str(), sso) {
            ("200", _) => (true, "the API sees the repository".to_string(), None),
            ("403", Some(s)) => verdict(&repo_api.on_sso, s.trim_start_matches("required; url=")),
            ("404", _) => verdict(&repo_api.on_not_found, ""),
            (s, _) => (false, format!("HTTP {s}"), None),
        };
        out.push(Finding {
            id: "api-repo",
            label: format!("API access to {path}"),
            ok,
            detail,
            fix,
        });
    }
}

#[cfg(test)]
mod host_tests {
    use super::remote_host;

    #[test]
    fn hosts_of_the_usual_remote_forms() {
        for (u, h) in [
            ("https://github.com/o/r.git", "github.com"),
            (
                "https://user:tok@GitLab.example.com:8443/o/r",
                "gitlab.example.com",
            ),
            ("git@github.com:o/r.git", "github.com"),
            ("ssh://git@git.example.com:22/o/r", "git.example.com"),
        ] {
            assert_eq!(remote_host(u).as_deref(), Some(h), "{u}");
        }
        assert_eq!(remote_host(""), None);
        assert_eq!(remote_host("/local/path/repo"), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(h: &str, c: Credential) -> GitHost {
        GitHost {
            host: h.into(),
            protocols: vec!["https".into()],
            credential: c,
            provider: None,
            username: None,
            ca_file: None,
            prefer_https: false,
            prefer_ssh: false,
        }
    }

    #[test]
    fn gitconfig_per_host() {
        let mut gl = host("gitlab.example.com", Credential::Xherd);
        gl.ca_file = Some("/certs/ca.pem".into());
        gl.prefer_https = true;
        gl.username = Some("me".into());
        let mut bb = host("bitbucket.org", Credential::None);
        bb.prefer_ssh = true;
        let t = managed_gitconfig(
            &[host("github.com", Credential::Provider), gl, bb],
            "/usr/local/bin/xherd",
            true,
            Some(("Ada \"A\" L", "ada@example.com")),
            &[],
            &|h| (h.host == "github.com").then(|| "!gh auth git-credential".to_string()),
        );
        assert!(
            t.contains("[user]\n\tname = \"Ada \\\"A\\\" L\"\n\temail = \"ada@example.com\"\n")
        );
        assert!(t.contains("[safe]\n\tdirectory = *\n"));
        assert!(t.contains(
            "[credential \"https://github.com\"]\n\thelper =\n\thelper = !gh auth git-credential\n"
        ));
        assert!(t.contains("\thelper = !'/usr/local/bin/xherd' credential\n\tusername = me\n"));
        assert!(
            t.contains("[http \"https://gitlab.example.com/\"]\n\tsslCAInfo = /certs/ca.pem\n")
        );
        assert!(t.contains(
            "[url \"https://gitlab.example.com/\"]\n\tinsteadOf = git@gitlab.example.com:\n"
        ));
        assert!(t.contains("[url \"git@bitbucket.org:\"]\n\tinsteadOf = https://bitbucket.org/\n"));
        assert!(!t.contains("[credential \"https://bitbucket.org\"]"));
        assert!(!managed_gitconfig(&[], "x", false, None, &[], &|_| None).contains("[safe]"));
        assert!(!managed_gitconfig(&[], "x", false, None, &[], &|_| None).contains("[user]"));
    }

    #[test]
    fn remotes() {
        let want = Some(("github.com".to_string(), "acme/app".to_string()));
        assert_eq!(parse_remote("https://github.com/acme/app.git"), want);
        assert_eq!(parse_remote("git@github.com:acme/app.git"), want);
        assert_eq!(parse_remote("ssh://git@github.com/acme/app"), want);
        assert_eq!(parse_remote("https://user@github.com/acme/app/"), want);
        assert_eq!(parse_remote("nonsense"), None);
    }

    fn providers() -> Vec<Provider> {
        let dir = tempfile::tempdir().unwrap();
        let plugins = xherd_plugin::Registry::load(xherd_plugin::builtins(), dir.path());
        plugins
            .of_kind(Kind::Host)
            .filter_map(|p| Some((p.name().to_string(), Git::of(&p.manifest).unwrap()?)))
            .collect()
    }

    #[test]
    fn token_users_come_from_the_providers() {
        let providers = providers();
        for (host, named, user) in [
            ("github.com", None, "x-access-token"),
            ("gitlab.com", None, "oauth2"),
            ("gitlab.example.com", None, "oauth2"),
            ("bitbucket.org", None, "x-token-auth"),
            ("git.example.com", None, "x-access-token"),
            // A binding beats the host name.
            ("git.example.com", Some("gitlab"), "oauth2"),
            ("gitlab.example.com", Some("github"), "x-access-token"),
        ] {
            assert_eq!(token_user_of(&providers, named, host), user, "{host}");
        }
    }

    #[test]
    fn a_host_is_bound_by_name_or_by_the_names_it_claims() {
        let providers = providers();
        let name = |named, host| {
            providers
                .iter()
                .find(|(_, g)| Some(g) == bound(&providers, named, host))
                .map(|(n, _)| n.as_str())
        };
        assert_eq!(name(None, "github.com"), Some("github"));
        assert_eq!(name(None, "git.example.com"), None);
        assert_eq!(name(Some("gitlab"), "git.example.com"), Some("gitlab"));
        assert_eq!(name(Some("nope"), "github.com"), None);
    }

    #[test]
    fn the_built_in_manifests_give_the_helpers_the_config_always_had() {
        let providers = providers();
        let mut gh = host("github.com", Credential::Provider);
        gh.provider = Some("github".into());
        let mut gl = host("gitlab.example.com", Credential::Provider);
        gl.provider = Some("gitlab".into());
        // No provider named: the host name decides.
        let auto = host("gitlab.com", Credential::Provider);
        let t = managed_gitconfig(
            &[gh, gl, auto, host("git.example.com", Credential::Provider)],
            "/usr/local/bin/xherd",
            false,
            None,
            &[],
            &|h| {
                bound(&providers, h.provider.as_deref(), &h.host)
                    .and_then(|g| g.credential.clone())
                    .and_then(|c| c.helper)
            },
        );
        assert!(t.contains(
            "[credential \"https://github.com\"]\n\thelper =\n\thelper = !gh auth git-credential\n"
        ));
        assert!(t.contains(
            "[credential \"https://gitlab.example.com\"]\n\thelper =\n\thelper = !glab auth git-credential\n"
        ));
        assert!(t.contains(
            "[credential \"https://gitlab.com\"]\n\thelper =\n\thelper = !glab auth git-credential\n"
        ));
        assert!(!t.contains("git.example.com\"]"));
    }
}
