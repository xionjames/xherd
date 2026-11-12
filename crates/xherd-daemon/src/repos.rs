//! Repositories for new agents (R-001, R-002, R-605): a local path, or a
//! remote URL that xherd clones once into `<workspace>/repos/<host>/<path>`
//! and fetches before each later use; and the list the New agent screen
//! offers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use xherd_core::Millis;
use xherd_core::new;
use xherd_git::Git;

use crate::create::{outside_mounts, refuse, resolve_base};
use crate::{ActionError, Daemon};

/// A clone of a large repository can take a while.
const CLONE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// A repository ready for a worktree.
#[derive(Debug, Clone)]
pub struct Located {
    /// Top level of the repository.
    pub path: PathBuf,
    /// Cloned by this call.
    pub cloned: bool,
    /// Something the user should know (e.g. the fetch failed).
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownRepo {
    pub path: String,
    /// Creation time of the newest agent on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<Millis>,
}

impl Daemon {
    /// A path or URL to a repository, cloning or fetching a URL first.
    pub(crate) async fn locate_repo(&self, input: &str) -> Result<Located, ActionError> {
        let input = input.trim();
        let git = Git::new("git");
        let mut found = Located {
            path: PathBuf::from(input),
            cloned: false,
            warning: None,
        };
        if new::is_url(input) {
            let dest = new::clone_path(&self.config.workspace_root(), input)?;
            found = clone_or_fetch(&git, input, &dest).await?;
        } else if !found.path.is_absolute() {
            return Err(refuse(
                "not_a_repo",
                format!("repo {input:?} must be an absolute path or a URL"),
            ));
        }
        found.path = match git.toplevel(&found.path).await {
            Ok(r) if found.path.is_dir() => r,
            _ => {
                return Err(refuse(
                    "not_a_repo",
                    format!("{} is not a git repository", found.path.display()),
                ));
            }
        };
        if self.sandbox {
            let extra = std::env::var("XHERD_EXTRA_PATHS").unwrap_or_default();
            outside_mounts(&found.path, &self.config.workspace_root(), &extra)?;
        }
        Ok(found)
    }

    /// `POST /repos/resolve`: the repository plus its branches and the base a
    /// new agent would get, for the New agent screen.
    pub async fn resolve_repo(&self, input: &str) -> Result<Value, ActionError> {
        let r = self.locate_repo(input).await?;
        let git = Git::new("git");
        let branches = git.branches(&r.path).await.unwrap_or_default();
        let profile = self.profile_for_repo(&r.path).await;
        let base = resolve_base(
            &git,
            &r.path,
            profile.as_ref().and_then(|p| p.base_branch.as_deref()),
        )
        .await
        .ok();
        Ok(json!({
            "path": r.path, "cloned": r.cloned, "warning": r.warning,
            "branches": branches, "base": base,
            // What a new profile for this repository would match: its
            // remote (`host/owner/repo`), else its folder.
            "suggestedMatch": match crate::profiles::repo_origin(&r.path).await {
                Some(o) => xherd_core::profile::normalize_url(&o),
                None => r.path.to_string_lossy().into_owned(),
            },
            "profile": profile.as_ref().map(|p| json!({
                "id": p.id, "name": p.name, "autonomy": p.autonomy,
                "permissionMode": p.permission_mode, "runtime": p.runtime,
            })),
        }))
    }

    /// `GET /repos`: repositories of earlier agents and xherd's clones, most
    /// recently used first.
    pub async fn known_repos(&self) -> Result<Vec<KnownRepo>, ActionError> {
        let agents = self
            .store
            .list_agents()
            .await
            .map_err(|e| refuse("store", e.to_string()))?;
        let mut used: BTreeMap<String, Option<Millis>> = BTreeMap::new();
        for a in &agents {
            if let Some(r) = &a.repo {
                let e = used.entry(r.clone()).or_default();
                *e = (*e).max(Some(a.created_at));
            }
        }
        let mut clones = Vec::new();
        scan(&self.config.workspace_root().join("repos"), 0, &mut clones);
        for c in clones {
            // Agents store git's real path (`/private/var` for `/var`).
            let c = std::fs::canonicalize(&c).unwrap_or(c);
            used.entry(c.to_string_lossy().into_owned()).or_default();
        }
        let mut out: Vec<KnownRepo> = used
            .into_iter()
            .filter(|(p, _)| Path::new(p).is_dir())
            .map(|(path, last_used)| KnownRepo { path, last_used })
            .collect();
        out.sort_by(|a, b| b.last_used.cmp(&a.last_used).then(a.path.cmp(&b.path)));
        Ok(out)
    }
}

/// Remove a folder git made: as the agent user, who owns what is inside it
/// (the daemon's user cannot delete those files, R-1203).
async fn remove_tree(dir: &Path) {
    use xherd_core::exec::{Cmd, run};
    let dir = dir.to_string_lossy().into_owned();
    let _ = run(Cmd::new(["rm", "-rf", "--", dir.as_str()]).as_agent()).await;
}

/// Reuse and fetch an existing clone, else clone into a temporary folder
/// beside `dest` and rename it, so a failed or concurrent clone never leaves
/// a half-made repository at `dest`.
async fn clone_or_fetch(git: &Git, url: &str, dest: &Path) -> Result<Located, ActionError> {
    if dest.exists() {
        if !git.is_repo(dest).await {
            return Err(refuse(
                "path_exists",
                format!("{} exists and is not a git repository", dest.display()),
            ));
        }
        let warning = git
            .fetch(dest, "origin")
            .await
            .err()
            .map(|e| format!("could not fetch {url} ({e}); using the clone as it is"));
        return Ok(Located {
            path: dest.to_path_buf(),
            cloned: false,
            warning,
        });
    }
    let parent = dest.parent().expect("clone_path has parents");
    std::fs::create_dir_all(parent)
        .map_err(|e| refuse("io", format!("{}: {e}", parent.display())))?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let name = dest
        .file_name()
        .map_or("repo".into(), |n| n.to_string_lossy());
    let tmp = parent.join(format!(".{name}.cloning-{nanos}"));
    let slow = git.clone().with_timeout(CLONE_TIMEOUT);
    if let Err(e) = slow.clone_repo(url, &tmp).await {
        remove_tree(&tmp).await;
        // git's own message, not the command line with the temporary folder.
        let e = match e {
            xherd_git::GitError::Failed { stderr, .. } => stderr,
            e => e.to_string(),
        };
        let host = parent
            .ancestors()
            .find(|a| a.parent().and_then(Path::file_name) == Some("repos".as_ref()))
            .and_then(Path::file_name)
            .map_or(String::new(), |h| h.to_string_lossy().into_owned());
        return Err(refuse(
            "clone_failed",
            format!(
                "cannot clone {url}: {e}. Check Settings → Git access for {host} \
                 (`xherd git doctor {host}`)"
            ),
        ));
    }
    if let Err(e) = std::fs::rename(&tmp, dest) {
        remove_tree(&tmp).await;
        // Another clone of the same URL finished first: use it.
        if !git.is_repo(dest).await {
            return Err(refuse("io", format!("{}: {e}", dest.display())));
        }
    }
    Ok(Located {
        path: dest.to_path_buf(),
        cloned: true,
        warning: None,
    })
}

/// Repositories under `repos/` (`<host>/<owner…>/<repo>`); hidden folders,
/// such as clones in progress, are skipped.
fn scan(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 6 {
        return;
    }
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if e.file_name().to_string_lossy().starts_with('.')
            || !e.file_type().is_ok_and(|t| t.is_dir())
        {
            continue;
        }
        if p.join(".git").exists() {
            out.push(p);
        } else {
            scan(&p, depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_nested_repos_only() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        for p in [
            "github.com/acme/app/.git",
            "gitlab.com/g/sub/lib/.git",
            "github.com/acme/.app.cloning-1/.git",
            "github.com/acme/app/inner/.git",
        ] {
            std::fs::create_dir_all(r.join(p)).unwrap();
        }
        let mut out = Vec::new();
        scan(r, 0, &mut out);
        out.sort();
        assert_eq!(
            out,
            [
                r.join("github.com/acme/app"),
                r.join("gitlab.com/g/sub/lib")
            ]
        );
    }
}
