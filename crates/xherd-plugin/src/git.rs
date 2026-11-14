//! Git providers (D24, R-689…R-693): what a `host` plugin declares about git
//! access in `data.git`. Transport (SSH agent, known hosts, token storage, the
//! managed gitconfig) stays in the daemon; the provider supplies the
//! credential helper, the browser login and the checks of its API.
//!
//! Everything here ends up in a git config file or a command line, so values
//! are validated when the manifest is read ([`Git::of`]): no control
//! characters (a newline in a helper would add arbitrary config lines), and
//! `{host}` / `{path}` are replaced only as plain text by the daemon, whose
//! host names and repository paths are already restricted.

use serde::Deserialize;

use crate::Manifest;

/// `data.git` of a host plugin.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Git {
    /// Host names the provider claims; `*` matches any run of characters
    /// (`github.com`, `*gitlab*`). Decides the token user and the API checks.
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default)]
    pub credential: Option<Credential>,
    /// argv of the browser login; `{host}` is replaced. Absent: paste a token.
    #[serde(default)]
    pub login: Option<Vec<String>>,
    #[serde(default)]
    pub api: Option<Api>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Credential {
    /// The git credential helper (`!gh auth git-credential`). Absent for a
    /// provider with no CLI: the host's token is kept by xherd.
    #[serde(default)]
    pub helper: Option<String>,
    /// The user name git sends with a token when none is configured.
    #[serde(default)]
    pub token_user: Option<String>,
}

/// The doctor's API checks for the hosts the provider claims.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Api {
    /// `{host}` is replaced.
    pub base: String,
    pub user: ApiUser,
    /// Optional: a check that the token sees a repository.
    #[serde(default)]
    pub repo: Option<ApiRepo>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiUser {
    pub path: String,
    /// Response header listing the token's scopes, and the scope it needs.
    #[serde(default)]
    pub scopes_header: Option<String>,
    #[serde(default)]
    pub require_scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiRepo {
    /// `{path}` is the repository's `owner/name`.
    pub path: String,
    /// Header that carries the SSO authorisation link on a 403.
    #[serde(default)]
    pub sso_header: Option<String>,
    pub on_sso: Verdict,
    pub on_not_found: Verdict,
}

/// What a failed check says; `fix` may use `{url}` (the SSO header's link).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Verdict {
    pub detail: String,
    #[serde(default)]
    pub fix: Option<String>,
}

fn plain(what: &str, s: &str) -> Result<(), String> {
    if s.chars().any(char::is_control) {
        Err(format!("{what} has a control character"))
    } else {
        Ok(())
    }
}

impl Git {
    /// `None` when the plugin declares no git access.
    pub fn of(m: &Manifest) -> Result<Option<Git>, String> {
        let Some(v) = m.data.get("git") else {
            return Ok(None);
        };
        let g: Git =
            serde_json::from_value(v.clone()).map_err(|e| format!("git {}: {e}", m.name))?;
        g.validate().map_err(|e| format!("git {}: {e}", m.name))?;
        Ok(Some(g))
    }

    fn validate(&self) -> Result<(), String> {
        for h in &self.hosts {
            if h.is_empty()
                || !h
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '*'))
            {
                return Err(format!("{h:?} is not a host pattern"));
            }
        }
        if let Some(c) = &self.credential {
            if let Some(h) = &c.helper {
                plain("credential.helper", h)?;
                if h.trim().is_empty() {
                    return Err("credential.helper is empty".into());
                }
            }
            if let Some(u) = &c.token_user {
                plain("credential.tokenUser", u)?;
            }
        }
        for a in self.login.iter().flatten() {
            plain("login", a)?;
        }
        if let Some(api) = &self.api {
            if !api.base.starts_with("https://") {
                return Err("api.base must be an https URL".into());
            }
            plain("api.base", &api.base)?;
            plain("api.user.path", &api.user.path)?;
            if let Some(r) = &api.repo {
                plain("api.repo.path", &r.path)?;
                for v in [&r.on_sso, &r.on_not_found] {
                    plain("api.repo verdict", &v.detail)?;
                    plain("api.repo verdict", v.fix.as_deref().unwrap_or(""))?;
                }
            }
        }
        Ok(())
    }

    /// The provider claims `host` (case-insensitive).
    pub fn claims(&self, host: &str) -> bool {
        self.hosts.iter().any(|p| host_matches(p, host))
    }
}

/// `*` matches any run of characters; everything else is literal.
pub fn host_matches(pattern: &str, host: &str) -> bool {
    let (p, h) = (pattern.to_ascii_lowercase(), host.to_ascii_lowercase());
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == h;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if h.len() < first.len() + last.len() || !h.starts_with(first) || !h.ends_with(last) {
        return false;
    }
    let mut rest = &h[first.len()..h.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

/// Replace `{host}` in every argument.
pub fn subst_host(argv: &[String], host: &str) -> Vec<String> {
    argv.iter().map(|a| a.replace("{host}", host)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest(git: serde_json::Value) -> Manifest {
        Manifest::parse(
            &json!({"name": "p", "kind": "host", "version": "0.1.0", "data": {"git": git}})
                .to_string(),
        )
        .unwrap()
    }

    #[test]
    fn patterns() {
        for (p, h, want) in [
            ("github.com", "github.com", true),
            ("github.com", "GitHub.com", true),
            ("github.com", "github.com.evil.io", false),
            ("github.com", "api.github.com", false),
            ("*gitlab*", "gitlab.com", true),
            ("*gitlab*", "my-gitlab.example.com", true),
            ("*gitlab*", "github.com", false),
            ("git.*.example.com", "git.eu.example.com", true),
            ("git.*.example.com", "git.eu.example.org", false),
            ("*.example.com", "a.example.com", true),
            ("*.example.com", "a.example.com.x", false),
        ] {
            assert_eq!(host_matches(p, h), want, "{p} vs {h}");
        }
    }

    #[test]
    fn reads_a_provider() {
        let g = Git::of(&manifest(json!({
            "hosts": ["gitlab.com"],
            "credential": {"helper": "!glab auth git-credential", "tokenUser": "oauth2"},
            "login": ["glab", "auth", "login", "--hostname", "{host}"],
            "api": {"base": "https://{host}/api/v4", "user": {"path": "/user"}}
        })))
        .unwrap()
        .unwrap();
        assert!(g.claims("gitlab.com") && !g.claims("github.com"));
        assert_eq!(
            subst_host(g.login.as_ref().unwrap(), "gitlab.com")[4],
            "gitlab.com"
        );
        let none =
            Manifest::parse(&json!({"name": "p", "kind": "host", "version": "0.1.0"}).to_string())
                .unwrap();
        assert_eq!(Git::of(&none).unwrap(), None);
    }

    #[test]
    fn refuses_what_would_inject_config_or_commands() {
        for bad in [
            json!({"credential": {"helper": "!x\n[core]\n\tsshCommand = evil"}}),
            json!({"credential": {"helper": "!x", "tokenUser": "a\nb"}}),
            json!({"login": ["gh", "a\nb"]}),
            json!({"hosts": ["bad host"]}),
            json!({"hosts": [""]}),
            json!({"api": {"base": "http://plain/", "user": {"path": "/u"}}}),
            json!({"unknown": 1}),
        ] {
            assert!(Git::of(&manifest(bad.clone())).is_err(), "{bad}");
        }
    }

    fn builtin(name: &str) -> Git {
        let m = crate::builtins()
            .into_iter()
            .find(|m| m.name == name)
            .unwrap();
        Git::of(&m).unwrap().unwrap()
    }

    /// GitHub's login goes through the plugin-login path (Welcome's Plugins
    /// step): a top-level `login` argv and an optional check named `login`.
    #[test]
    fn github_has_a_plugin_login() {
        let m = crate::builtins()
            .into_iter()
            .find(|m| m.name == "github")
            .unwrap();
        assert_eq!(
            m.login.as_deref().unwrap(),
            [
                "gh",
                "auth",
                "login",
                "--hostname",
                "github.com",
                "--git-protocol",
                "https",
                "--web"
            ]
        );
        let check = m
            .checks
            .iter()
            .find(|c| c.name == crate::manifest::LOGIN_CHECK)
            .unwrap();
        assert!(check.optional);
    }

    /// What `git_access.rs` hardcoded before D24 step 1, now in the manifests.
    #[test]
    fn the_built_in_providers_say_what_the_daemon_used_to() {
        let gh = builtin("github");
        assert!(
            gh.claims("github.com") && !gh.claims("gitlab.com") && !gh.claims("api.github.com")
        );
        let c = gh.credential.as_ref().unwrap();
        assert_eq!(c.helper.as_deref(), Some("!gh auth git-credential"));
        assert_eq!(c.token_user.as_deref(), Some("x-access-token"));
        assert_eq!(
            subst_host(gh.login.as_ref().unwrap(), "github.com"),
            [
                "gh",
                "auth",
                "login",
                "--hostname",
                "github.com",
                "--git-protocol",
                "https",
                "--web"
            ]
        );
        let api = gh.api.as_ref().unwrap();
        assert_eq!(api.base, "https://api.github.com");
        assert_eq!(api.user.path, "/user");
        assert_eq!(api.user.scopes_header.as_deref(), Some("x-oauth-scopes"));
        assert_eq!(api.user.require_scope.as_deref(), Some("repo"));
        let repo = api.repo.as_ref().unwrap();
        assert_eq!(repo.path, "/repos/{path}");
        assert_eq!(repo.sso_header.as_deref(), Some("x-github-sso"));

        let gl = builtin("gitlab");
        assert!(
            gl.claims("gitlab.com") && gl.claims("gitlab.example.com") && !gl.claims("github.com")
        );
        let c = gl.credential.as_ref().unwrap();
        assert_eq!(c.helper.as_deref(), Some("!glab auth git-credential"));
        assert_eq!(c.token_user.as_deref(), Some("oauth2"));
        assert_eq!(
            subst_host(gl.login.as_ref().unwrap(), "gitlab.com"),
            ["glab", "auth", "login", "--hostname", "gitlab.com"]
        );
        let api = gl.api.as_ref().unwrap();
        assert_eq!(api.base, "https://{host}/api/v4");
        assert!(api.user.scopes_header.is_none() && api.repo.is_none());

        let bb = builtin("bitbucket");
        assert!(
            bb.claims("bitbucket.org")
                && bb.claims("bitbucket.example.com")
                && !bb.claims("github.com")
        );
        let c = bb.credential.as_ref().unwrap();
        assert!(c.helper.is_none());
        assert_eq!(c.token_user.as_deref(), Some("x-token-auth"));
        assert!(bb.login.is_none() && bb.api.is_none());
    }
}
