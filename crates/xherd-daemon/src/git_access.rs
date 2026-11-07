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
use xherd_core::config::{Config, Credential, GIT_SECRETS, GitHost, Secrets};
use xherd_core::exec::{Cmd, run};
use xherd_core::paths::write_atomic;

use crate::checks::Fix;
use crate::{ActionError, Daemon};

/// tmux session of the browser login (`gh` / `glab` device flow).
pub const LOGIN_SESSION: &str = "xherd-setup-git-login";
pub const LOGIN_ID: &str = "git-login";

fn fail(code: &'static str, message: impl Into<String>) -> ActionError {
    ActionError {
        code,
        message: message.into(),
    }
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The managed git config (`~/.config/xherd/gitconfig`, included from
/// `~/.gitconfig` in the container). Pure, so it is tested directly.
pub fn managed_gitconfig(hosts: &[GitHost], xherd_bin: &str, sandbox: bool) -> String {
    let mut out =
        String::from("# Managed by xherd (Settings → Git access); rewritten on every change.\n");
    if sandbox {
        // One user owns every file in the container (design §8.4).
        out.push_str("[safe]\n\tdirectory = *\n");
    }
    for h in hosts {
        let helper = match h.credential {
            Credential::None => None,
            // gh reads the host from git on stdin; it has no --hostname flag.
            Credential::Gh => Some("!gh auth git-credential".to_string()),
            Credential::Glab => Some("!glab auth git-credential".to_string()),
            Credential::Xherd => Some(format!("!{} credential", quote(xherd_bin))),
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

/// The username git sends with a token when none is configured.
fn token_user(host: &str) -> &'static str {
    if host.contains("gitlab") {
        "oauth2"
    } else if host.contains("bitbucket") {
        "x-token-auth"
    } else {
        "x-access-token"
    }
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

impl Daemon {
    fn live_config(&self) -> Config {
        Config::load(&self.paths.config())
            .map(|(c, _)| c)
            .unwrap_or_else(|_| (*self.config).clone())
    }

    fn live_secrets(&self) -> Secrets {
        Secrets::load(&self.paths.secrets()).unwrap_or_else(|_| (*self.secrets).clone())
    }

    /// The managed git config: in the container `~/.config/xherd/gitconfig`
    /// (included from `~/.gitconfig` by `xherd init`); natively a file in the
    /// data root that the user may include (xherd never edits their config).
    pub fn gitconfig_path(&self) -> Option<PathBuf> {
        if self.sandbox {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/xherd/gitconfig"))
        } else {
            Some(self.paths.root.join("gitconfig"))
        }
    }

    /// Rewrite the managed git config (at start and on every change).
    pub fn write_gitconfig(&self) -> Result<(), ActionError> {
        let Some(p) = self.gitconfig_path() else {
            return Ok(());
        };
        let text = managed_gitconfig(&self.live_config().git_hosts, &self.xherd_bin, self.sandbox);
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
        serde_json::json!({
            "hosts": hosts,
            "sandbox": self.sandbox,
            "gitconfig": self.gitconfig_path(),
        })
    }

    /// `PUT /git/hosts`: save hosts and tokens, rewrite the managed config.
    pub fn set_git_hosts(&self, u: HostsUpdate) -> Result<Value, ActionError> {
        let mut seen = std::collections::BTreeSet::new();
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
            h.username
                .clone()
                .unwrap_or_else(|| token_user(host).to_string()),
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
        let home = std::env::var("HOME").map_err(|_| fail("io", "HOME is not set"))?;
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
        let mut text = std::fs::read_to_string(&file).unwrap_or_default();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        for l in lines {
            text.push_str(&l);
            text.push('\n');
        }
        if let Some(d) = file.parent() {
            std::fs::create_dir_all(d).map_err(|e| fail("io", e.to_string()))?;
        }
        write_atomic(&file, text.as_bytes(), Some(0o644))
            .map_err(|e| fail("io", format!("{}: {e}", file.display())))?;
        tracing::info!(host, fingerprint, "host key accepted");
        Ok(())
    }

    /// Start `gh auth login --web` (or glab) in a tmux session the UI shows
    /// (`/ws/setup/git-login`); the code to type appears there (R-686).
    pub async fn start_git_login(&self, host: &str) -> Result<(), ActionError> {
        let c = self.live_config();
        let cred = c
            .git_hosts
            .iter()
            .find(|h| h.host == host)
            .map(|h| h.credential)
            .unwrap_or(if host.contains("gitlab") {
                Credential::Glab
            } else {
                Credential::Gh
            });
        let argv: Vec<String> = match cred {
            Credential::Gh => [
                "gh",
                "auth",
                "login",
                "--hostname",
                host,
                "--git-protocol",
                "https",
                "--web",
            ]
            .map(String::from)
            .to_vec(),
            Credential::Glab => ["glab", "auth", "login", "--hostname", host]
                .map(String::from)
                .to_vec(),
            _ => {
                return Err(fail(
                    "not_fixable",
                    format!("{host} does not log in through gh or glab; paste a token instead"),
                ));
            }
        };
        let tmux = xherd_term::Tmux::new(self.config.tmux_socket.clone());
        if tmux.has_session(LOGIN_SESSION).await {
            let _ = tmux.kill_session(LOGIN_SESSION).await;
        }
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()));
        tmux.new_session(LOGIN_SESSION, &home, &[], &argv)
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
        if protocols.iter().any(|p| p == "https") {
            self.doctor_https(host, repo, &mut out).await;
        }
        if let Some(url) = repo {
            let r = sh(Cmd::new(["git", "ls-remote", "--heads", url])
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GCM_INTERACTIVE", "never")
                .env(
                    "GIT_SSH_COMMAND",
                    "ssh -o BatchMode=yes -o ConnectTimeout=10",
                ))
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
        let keys = sh(Cmd::new(["ssh-add", "-l"])).await;
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
        let known = sh(Cmd::new(["ssh", "-G", host])).await;
        let files: Vec<String> = known
            .out
            .lines()
            .filter_map(|l| l.strip_prefix("userknownhostsfile "))
            .flat_map(|l| l.split_whitespace().map(str::to_string))
            .collect();
        let name = host.split(':').next().unwrap_or(host);
        let mut is_known = false;
        for f in &files {
            let f = f.replacen('~', &std::env::var("HOME").unwrap_or_default(), 1);
            if sh(Cmd::new(["ssh-keygen", "-F", name, "-f", &f]))
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
        ]))
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
        let helper = sh(Cmd::new([
            "git",
            "config",
            "--get-urlmatch",
            "credential.helper",
            &url,
        ]))
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
            .env("GCM_INTERACTIVE", "never"))
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
        let api = if host == "github.com" {
            Some("https://api.github.com".to_string())
        } else if host.contains("gitlab") {
            Some(format!("https://{host}/api/v4"))
        } else {
            None
        };
        let Some(api) = api else { return };
        let gitlab = !api.contains("api.github.com");
        let get = |path: String| {
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
                &path,
            ])
            .stdin(format!("Authorization: Bearer {token}\n"))
            .redact(vec![token.clone()])
        };
        let me = sh(get(format!("{api}/user"))).await;
        let status = me
            .out
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .to_string();
        let scopes = me.out.lines().find_map(|l| {
            l.to_lowercase()
                .strip_prefix("x-oauth-scopes:")
                .map(|s| s.trim().to_string())
        });
        let mut ok = status == "200";
        let mut detail = if ok {
            "token accepted".to_string()
        } else {
            format!("HTTP {status} {}", last_line(&me.err))
        };
        if ok
            && !gitlab
            && let Some(s) = &scopes
            && !s.split(',').any(|x| x.trim() == "repo")
        {
            ok = false;
            detail = format!("the token has scopes [{s}] but needs repo");
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
        if !ok || gitlab {
            return;
        }
        let Some((_, path)) = repo.and_then(parse_remote) else {
            return;
        };
        let r = sh(get(format!("{api}/repos/{path}"))).await;
        let status = r
            .out
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .to_string();
        let sso = r.out.lines().find_map(|l| {
            l.to_lowercase()
                .starts_with("x-github-sso:")
                .then(|| l.split_once(':').map_or("", |x| x.1).trim().to_string())
        });
        let (ok, detail, fix) = match (status.as_str(), sso) {
            ("200", _) => (true, "the API sees the repository".to_string(), None),
            ("403", Some(s)) => (
                false,
                "the organisation uses SSO and the token is not authorised for it".to_string(),
                cmd_fix(format!("authorise the token for SSO: {}", s.trim_start_matches("required; url="))),
            ),
            ("404", _) => (
                false,
                "the token cannot see the repository: no access, or the organisation does not allow this OAuth app".to_string(),
                cmd_fix("ask an organisation owner to approve the app, or use a token with access"),
            ),
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
mod tests {
    use super::*;

    fn host(h: &str, c: Credential) -> GitHost {
        GitHost {
            host: h.into(),
            protocols: vec!["https".into()],
            credential: c,
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
            &[host("github.com", Credential::Gh), gl, bb],
            "/usr/local/bin/xherd",
            true,
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
        assert!(!managed_gitconfig(&[], "x", false).contains("[safe]"));
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

    #[test]
    fn token_users() {
        assert_eq!(token_user("github.com"), "x-access-token");
        assert_eq!(token_user("gitlab.example.com"), "oauth2");
    }
}
