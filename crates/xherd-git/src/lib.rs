//! git wrappers. Every call goes through `xherd_core::exec::run` with
//! `GIT_TERMINAL_PROMPT=0`, so a missing credential fails instead of hanging.

use std::path::{Path, PathBuf};
use std::time::Duration;

use xherd_core::exec::{Cmd, ExecError, run};

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error(transparent)]
    Exec(#[from] ExecError),
    #[error("git {args} failed: {stderr}")]
    Failed { args: String, stderr: String },
    #[error("git {args} timed out")]
    Timeout { args: String },
}

pub type Result<T> = std::result::Result<T, GitError>;

/// For calls that reach a remote: SSH fails instead of asking (host key,
/// passphrase), like `GIT_TERMINAL_PROMPT=0` does for HTTPS.
const NO_PROMPT: &[(&str, &str)] = &[
    (
        "GIT_SSH_COMMAND",
        "ssh -o BatchMode=yes -o ConnectTimeout=20",
    ),
    ("GCM_INTERACTIVE", "never"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    /// Short branch name; `None` when detached or bare.
    pub branch: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Git {
    bin: String,
    timeout: Duration,
}

impl Default for Git {
    fn default() -> Git {
        Git {
            bin: "git".into(),
            timeout: Duration::from_secs(120),
        }
    }
}

impl Git {
    pub fn new(bin: impl Into<String>) -> Git {
        Git {
            bin: bin.into(),
            ..Git::default()
        }
    }

    pub fn with_timeout(mut self, t: Duration) -> Git {
        self.timeout = t;
        self
    }

    /// Run git in `dir`; returns stdout with the trailing newline trimmed.
    pub async fn run(&self, dir: &Path, args: &[&str]) -> Result<String> {
        self.run_env(dir, &[], args).await
    }

    async fn run_env(&self, dir: &Path, env: &[(&str, &str)], args: &[&str]) -> Result<String> {
        let mut argv = vec![self.bin.clone()];
        argv.extend(args.iter().map(|s| s.to_string()));
        let mut cmd = Cmd::new(argv)
            .cwd(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .timeout(self.timeout);
        for (k, v) in env {
            cmd = cmd.env(*k, *v);
        }
        let out = run(cmd).await?;
        let joined = args.join(" ");
        if out.timed_out {
            return Err(GitError::Timeout { args: joined });
        }
        if !out.success() {
            return Err(GitError::Failed {
                args: joined,
                stderr: out.stderr.trim().to_string(),
            });
        }
        Ok(out.stdout_str().trim_end_matches('\n').to_string())
    }

    pub async fn is_repo(&self, dir: &Path) -> bool {
        self.run(dir, &["rev-parse", "--git-dir"]).await.is_ok()
    }

    pub async fn toplevel(&self, dir: &Path) -> Result<PathBuf> {
        Ok(PathBuf::from(
            self.run(dir, &["rev-parse", "--show-toplevel"]).await?,
        ))
    }

    /// The shared `.git` folder of a repo or any of its worktrees (absolute).
    pub async fn common_dir(&self, dir: &Path) -> Result<PathBuf> {
        let p = PathBuf::from(self.run(dir, &["rev-parse", "--git-common-dir"]).await?);
        Ok(if p.is_absolute() { p } else { dir.join(p) })
    }

    pub async fn rev_parse(&self, dir: &Path, rev: &str) -> Result<String> {
        self.run(
            dir,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{rev}^{{commit}}"),
            ],
        )
        .await
    }

    pub async fn branch_exists(&self, dir: &Path, branch: &str) -> bool {
        self.run(
            dir,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )
        .await
        .is_ok()
    }

    pub async fn current_branch(&self, dir: &Path) -> Option<String> {
        self.run(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .await
            .ok()
    }

    /// `origin/HEAD`'s target as a short name (`origin/main`), if known.
    pub async fn remote_default_branch(&self, dir: &Path, remote: &str) -> Option<String> {
        self.run(
            dir,
            &[
                "symbolic-ref",
                "--quiet",
                "--short",
                &format!("refs/remotes/{remote}/HEAD"),
            ],
        )
        .await
        .ok()
    }

    pub async fn fetch(&self, dir: &Path, remote: &str) -> Result<()> {
        self.run_env(dir, NO_PROMPT, &["fetch", "--quiet", "--prune", remote])
            .await
            .map(drop)
    }

    /// `git clone <url> <dest>`, run in `dest`'s parent (which must exist).
    pub async fn clone_repo(&self, url: &str, dest: &Path) -> Result<()> {
        let parent = dest.parent().unwrap_or(Path::new("/"));
        let dest = dest.to_string_lossy();
        self.run_env(parent, NO_PROMPT, &["clone", "--quiet", "--", url, &dest])
            .await
            .map(drop)
    }

    /// Local branches and `origin`'s, by short name (`main`, `origin/main`),
    /// without `origin/HEAD`.
    pub async fn branches(&self, dir: &Path) -> Result<Vec<String>> {
        let out = self
            .run(
                dir,
                &[
                    "for-each-ref",
                    "--format=%(refname:short)",
                    "refs/heads",
                    "refs/remotes/origin",
                ],
            )
            .await?;
        Ok(out
            .lines()
            .filter(|b| *b != "origin" && *b != "origin/HEAD")
            .map(str::to_string)
            .collect())
    }

    /// `git worktree add --no-track -b <branch> <path> <base>` (R-004): no upstream,
    /// so a plain `git push` cannot land on the base branch.
    pub async fn worktree_add(
        &self,
        repo: &Path,
        path: &Path,
        branch: &str,
        base: &str,
    ) -> Result<()> {
        let p = path.to_string_lossy();
        self.run(
            repo,
            &[
                "worktree",
                "add",
                "--quiet",
                "--no-track",
                "-b",
                branch,
                &p,
                base,
            ],
        )
        .await
        .map(drop)
    }

    pub async fn worktree_remove(&self, repo: &Path, path: &Path, force: bool) -> Result<()> {
        let p = path.to_string_lossy();
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&p);
        self.run(repo, &args).await.map(drop)
    }

    pub async fn worktree_list(&self, repo: &Path) -> Result<Vec<Worktree>> {
        let out = self.run(repo, &["worktree", "list", "--porcelain"]).await?;
        Ok(parse_worktrees(&out))
    }

    /// Unified diff of the working tree against `base` (or the index when `None`).
    pub async fn diff(&self, dir: &Path, base: Option<&str>, paths: &[&str]) -> Result<String> {
        let mut args = vec!["diff", "--no-color", "--no-ext-diff"];
        if let Some(b) = base {
            args.push(b);
        }
        args.push("--");
        args.extend_from_slice(paths);
        self.run(dir, &args).await
    }

    /// Files changed in the working tree relative to `base`, plus untracked files.
    pub async fn changed_files(&self, dir: &Path, base: &str) -> Result<Vec<String>> {
        let tracked = self.run(dir, &["diff", "--name-only", base, "--"]).await?;
        let untracked = self
            .run(dir, &["ls-files", "--others", "--exclude-standard"])
            .await?;
        let mut files: Vec<String> = tracked
            .lines()
            .chain(untracked.lines())
            .map(str::to_string)
            .collect();
        files.sort();
        files.dedup();
        Ok(files)
    }

    /// Unified diff of HEAD against `paths` as they are in the working tree,
    /// as if exactly these files were staged: new (even untracked or ignored)
    /// and deleted files included, paths taken literally. A temporary index is
    /// used, so the worktree's own index is never touched. Paths in neither
    /// HEAD nor the working tree are skipped.
    pub async fn diff_proposed(&self, dir: &Path, paths: &[String]) -> Result<String> {
        if paths.is_empty() {
            return Ok(String::new());
        }
        let index = std::env::temp_dir().join(format!("xherd-index-{}", xherd_core::new_id()));
        let idx = index.to_string_lossy().into_owned();
        let env = [("GIT_INDEX_FILE", idx.as_str())];
        let lit = "--literal-pathspecs";
        let res = async {
            self.run_env(dir, &env, &["read-tree", "HEAD"]).await?;
            let mut args = vec![lit, "ls-files", "-z", "--"];
            args.extend(paths.iter().map(String::as_str));
            let tracked = self.run_env(dir, &env, &args).await?;
            let tracked: Vec<&str> = tracked.split('\0').collect();
            let present: Vec<&str> = paths
                .iter()
                .map(String::as_str)
                .filter(|p| tracked.contains(p) || dir.join(p).symlink_metadata().is_ok())
                .collect();
            if present.is_empty() {
                return Ok(String::new());
            }
            let mut args = vec![lit, "add", "--all", "--force", "--"];
            args.extend(&present);
            self.run_env(dir, &env, &args).await?;
            let mut args = vec![
                lit,
                "diff",
                "--cached",
                "--no-color",
                "--no-ext-diff",
                "HEAD",
                "--",
            ];
            args.extend(&present);
            self.run_env(dir, &env, &args).await
        }
        .await;
        let _ = std::fs::remove_file(&index);
        res
    }

    /// Commit exactly `paths` (new, changed and deleted files, taken literally)
    /// with `message`, whatever else is staged; returns the new sha. Hooks run
    /// (never `--no-verify`), so a failing hook is an error with its output.
    pub async fn commit_paths(
        &self,
        dir: &Path,
        paths: &[String],
        message: &str,
    ) -> Result<String> {
        let lit = "--literal-pathspecs";
        let mut add = vec![lit, "add", "--all", "--"];
        add.extend(paths.iter().map(String::as_str));
        self.run(dir, &add).await?;
        let mut commit = vec![lit, "commit", "--only", "-m", message, "--"];
        commit.extend(paths.iter().map(String::as_str));
        self.run(dir, &commit).await?;
        self.rev_parse(dir, "HEAD").await
    }

    /// Files touched by one commit.
    pub async fn commit_files(&self, dir: &Path, sha: &str) -> Result<Vec<String>> {
        let out = self
            .run(
                dir,
                &[
                    "diff-tree",
                    "--no-commit-id",
                    "--name-only",
                    "-r",
                    "--root",
                    sha,
                ],
            )
            .await?;
        Ok(out.lines().map(str::to_string).collect())
    }

    pub async fn is_ancestor(&self, dir: &Path, ancestor: &str, of: &str) -> bool {
        self.run(dir, &["merge-base", "--is-ancestor", ancestor, of])
            .await
            .is_ok()
    }

    /// Commits on `branch` not in `not` (e.g. unpushed work).
    pub async fn count_commits(&self, dir: &Path, branch: &str, not: &str) -> Result<usize> {
        let out = self
            .run(dir, &["rev-list", "--count", branch, "--not", not])
            .await?;
        Ok(out.trim().parse().unwrap_or(0))
    }

    pub async fn ls_remote(&self, dir: &Path, remote: &str) -> Result<Vec<(String, String)>> {
        let out = self.run(dir, &["ls-remote", remote]).await?;
        Ok(out
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .map(|(sha, r)| (sha.to_string(), r.to_string()))
            .collect())
    }

    /// `git bundle create <out> <branch> [--not <base>]`.
    pub async fn bundle_create(
        &self,
        dir: &Path,
        out: &Path,
        branch: &str,
        not: Option<&str>,
    ) -> Result<()> {
        let o = out.to_string_lossy();
        let mut args = vec!["bundle", "create", "--quiet", &o, branch];
        if let Some(n) = not {
            args.extend(["--not", n]);
        }
        self.run(dir, &args).await.map(drop)
    }

    pub async fn bundle_verify(&self, dir: &Path, bundle: &Path) -> Result<()> {
        let b = bundle.to_string_lossy();
        self.run(dir, &["bundle", "verify", "--quiet", &b])
            .await
            .map(drop)
    }

    /// Append patterns to `.git/info/exclude` of the repo `dir` belongs to,
    /// skipping ones already present (R-005b).
    pub async fn add_excludes(&self, dir: &Path, patterns: &[&str]) -> Result<()> {
        let file = self.common_dir(dir).await?.join("info").join("exclude");
        let existing = std::fs::read_to_string(&file).unwrap_or_default();
        let have: std::collections::HashSet<&str> = existing.lines().map(str::trim).collect();
        let mut add = String::new();
        for p in patterns.iter().filter(|p| !have.contains(*p)) {
            add.push_str(p);
            add.push('\n');
        }
        if add.is_empty() {
            return Ok(());
        }
        let mut content = existing.clone();
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&add);
        xherd_core::paths::write_atomic(&file, content.as_bytes(), None).map_err(|e| {
            GitError::Failed {
                args: "info/exclude".into(),
                stderr: e.to_string(),
            }
        })
    }
}

fn parse_worktrees(porcelain: &str) -> Vec<Worktree> {
    let mut out = Vec::new();
    let mut cur: Option<Worktree> = None;
    for line in porcelain.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            if let Some(w) = cur.take() {
                out.push(w);
            }
            cur = Some(Worktree {
                path: PathBuf::from(p),
                head: None,
                branch: None,
            });
        } else if let Some(w) = cur.as_mut() {
            if let Some(h) = line.strip_prefix("HEAD ") {
                w.head = Some(h.to_string());
            } else if let Some(b) = line.strip_prefix("branch ") {
                w.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
            }
        }
    }
    out.extend(cur);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain() {
        let s = "worktree /r\nHEAD abc\nbranch refs/heads/main\n\nworktree /w/SC-1\nHEAD def\ndetached\n";
        let w = parse_worktrees(s);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].branch.as_deref(), Some("main"));
        assert_eq!(w[1].branch, None);
        assert_eq!(w[1].head.as_deref(), Some("def"));
    }
}
