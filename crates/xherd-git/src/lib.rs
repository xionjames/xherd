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
    /// xherd itself declined the operation (the text says why).
    #[error("{0}")]
    Refused(String),
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

/// Working-tree files larger than this are left out of [`Git::diff_proposed`]
/// (hashing them is slow, and their diff would pass `exec::run`'s 4 MiB
/// output cap anyway).
pub const DIFF_FILE_LIMIT: u64 = 8 * 1024 * 1024;

/// Starts the line [`Git::diff_proposed`] writes for a file it left out:
/// `Not shown: <path> is <size>, over the <limit> limit`.
pub const TOO_LARGE: &str = "Not shown: ";

/// `6.0 GiB`, `512 B`.
fn human_size(n: u64) -> String {
    let mut v = n as f64;
    let mut unit = "B";
    for u in ["KiB", "MiB", "GiB", "TiB"] {
        if v < 1024.0 {
            break;
        }
        v /= 1024.0;
        unit = u;
    }
    if unit == "B" {
        format!("{n} B")
    } else {
        format!("{v:.1} {unit}")
    }
}

/// What [`Git::blob_at`] found at a path of a revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobAt {
    /// Nothing at that path.
    Missing,
    /// A symlink, a submodule or a directory.
    NotFile,
    /// A regular file larger than the limit (its size).
    TooLarge(u64),
    Found {
        oid: String,
        bytes: Vec<u8>,
    },
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

    /// Run git in `dir`; returns stdout exactly as written (file contents, e.g. `show`).
    pub async fn run_raw(&self, dir: &Path, args: &[&str]) -> Result<String> {
        self.run_raw_env(dir, &[], args).await
    }

    async fn run_env(&self, dir: &Path, env: &[(&str, &str)], args: &[&str]) -> Result<String> {
        Ok(self
            .run_raw_env(dir, env, args)
            .await?
            .trim_end_matches('\n')
            .to_string())
    }

    async fn run_raw_env(&self, dir: &Path, env: &[(&str, &str)], args: &[&str]) -> Result<String> {
        Ok(self.exec(dir, env, args, None, None).await?.stdout_str())
    }

    /// Run git in `dir` with `stdin`; returns stdout as raw bytes (blobs,
    /// hashes of binary files). Output over `max` bytes is an error, not a
    /// silent cut: `exec::run` caps output at 4 MiB unless told otherwise.
    pub async fn run_bytes(
        &self,
        dir: &Path,
        args: &[&str],
        max: usize,
        stdin: Option<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        let out = self.exec(dir, &[], args, Some(max), stdin).await?;
        if out.truncated {
            return Err(GitError::Refused(format!("output is over {max} bytes")));
        }
        Ok(out.stdout)
    }

    async fn exec(
        &self,
        dir: &Path,
        env: &[(&str, &str)],
        args: &[&str],
        max_output: Option<usize>,
        stdin: Option<Vec<u8>>,
    ) -> Result<xherd_core::exec::Output> {
        let mut argv = vec![self.bin.clone()];
        argv.extend(args.iter().map(|s| s.to_string()));
        let mut cmd = Cmd::new(argv)
            .cwd(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .timeout(self.timeout)
            // Git runs as the agent user (R-1203), who owns the repository.
            .as_agent();
        for (k, v) in env {
            cmd = cmd.env(*k, *v);
        }
        if let Some(m) = max_output {
            cmd.max_output = m;
        }
        cmd.stdin = stdin;
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
        Ok(out)
    }

    /// The file `path` (a literal repository-relative path) at `rev`, read
    /// from the object database: no checkout, no filters, no symlink followed.
    pub async fn blob_at(&self, dir: &Path, rev: &str, path: &str, max: u64) -> Result<BlobAt> {
        let listing = self
            .run_bytes(
                dir,
                &["--literal-pathspecs", "ls-tree", "-z", rev, "--", path],
                64 * 1024,
                None,
            )
            .await?;
        let listing = String::from_utf8_lossy(&listing);
        let Some(entry) = listing.split('\0').find(|e| !e.is_empty()) else {
            return Ok(BlobAt::Missing);
        };
        let Some((meta, _)) = entry.split_once('\t') else {
            return Ok(BlobAt::Missing);
        };
        let mut f = meta.split(' ');
        let (mode, kind, oid) = (f.next(), f.next(), f.next().unwrap_or(""));
        if kind != Some("blob") || !matches!(mode, Some("100644" | "100755")) {
            return Ok(BlobAt::NotFile);
        }
        let size: u64 = self
            .run(dir, &["cat-file", "-s", oid])
            .await?
            .trim()
            .parse()
            .unwrap_or(u64::MAX);
        if size > max {
            return Ok(BlobAt::TooLarge(size));
        }
        let bytes = self
            .run_bytes(dir, &["cat-file", "blob", oid], max as usize, None)
            .await?;
        Ok(BlobAt::Found {
            oid: oid.to_string(),
            bytes,
        })
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

    /// Whether `head` merges into `base` without conflicts, worked out by
    /// `git merge-tree --write-tree` (git ≥ 2.38; the worktree and index are
    /// not touched). `None` when git cannot tell (older git, unknown refs).
    pub async fn merges_cleanly(&self, dir: &Path, base: &str, head: &str) -> Option<bool> {
        let cmd = Cmd::new([
            self.bin.as_str(),
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            base,
            head,
        ])
        .cwd(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .timeout(self.timeout)
        .as_agent();
        match run(cmd).await.ok()? {
            o if o.timed_out => None,
            o => match o.code {
                Some(0) => Some(true),
                // A conflict prints the tree; a bad ref also exits 1, silently.
                Some(1) if !o.stdout.iter().all(u8::is_ascii_whitespace) => Some(false),
                _ => None,
            },
        }
    }

    pub async fn fetch(&self, dir: &Path, remote: &str) -> Result<()> {
        self.run_env(dir, NO_PROMPT, &["fetch", "--quiet", "--prune", remote])
            .await
            .map(drop)
    }

    /// Whether the repository has a remote with this name.
    pub async fn has_remote(&self, dir: &Path, remote: &str) -> bool {
        self.run(dir, &["remote"])
            .await
            .is_ok_and(|out| out.lines().any(|l| l.trim() == remote))
    }

    /// `git init` with `branch` as the first branch (the folder must exist).
    pub async fn init(&self, dir: &Path, branch: &str) -> Result<()> {
        self.run(dir, &["init", "--quiet", "-b", branch])
            .await
            .map(drop)
    }

    /// An empty commit on the current branch (a new repository has no `HEAD`
    /// until it has one); returns its sha.
    pub async fn commit_empty(&self, dir: &Path, message: &str) -> Result<String> {
        self.run(dir, &["commit", "--quiet", "--allow-empty", "-m", message])
            .await?;
        self.rev_parse(dir, "HEAD").await
    }

    pub async fn remote_add(&self, dir: &Path, name: &str, url: &str) -> Result<()> {
        if name.starts_with('-') || url.starts_with('-') {
            return Err(GitError::Refused(
                "a remote name or URL cannot start with '-'".into(),
            ));
        }
        self.run(dir, &["remote", "add", name, url]).await.map(drop)
    }

    pub async fn remote_remove(&self, dir: &Path, name: &str) -> Result<()> {
        self.run(dir, &["remote", "remove", name]).await.map(drop)
    }

    /// Pushes `branch` to `remote` and sets it as upstream. Never forced: a
    /// remote with other history refuses and git's message is the error.
    pub async fn push_branch(&self, dir: &Path, remote: &str, branch: &str) -> Result<()> {
        self.run_env(
            dir,
            NO_PROMPT,
            &["push", "--quiet", "--set-upstream", remote, branch],
        )
        .await
        .map(drop)
    }

    /// Pushes `branch` to `remote` without touching its upstream setting.
    /// Never forced: a diverged remote refuses and git's message is the error.
    pub async fn push(&self, dir: &Path, remote: &str, branch: &str) -> Result<()> {
        if branch.starts_with('-') || remote.starts_with('-') {
            return Err(GitError::Refused(
                "a remote or branch cannot start with '-'".into(),
            ));
        }
        self.run_env(dir, NO_PROMPT, &["push", "--quiet", remote, branch])
            .await
            .map(drop)
    }

    /// The commits of `branch` that `refs/remotes/origin/<branch>` lacks
    /// (shas, newest first), from local refs only (no fetch). `None` when
    /// there is no such tracking ref.
    pub async fn unpushed(&self, dir: &Path, branch: &str) -> Result<Option<Vec<String>>> {
        if branch.starts_with('-') {
            return Err(GitError::Refused("a branch cannot start with '-'".into()));
        }
        let tracking = format!("refs/remotes/origin/{branch}");
        if self
            .run(dir, &["rev-parse", "--verify", "--quiet", &tracking])
            .await
            .is_err()
        {
            return Ok(None);
        }
        let local = format!("refs/heads/{branch}");
        let out = self
            .run(dir, &["rev-list", &local, "--not", &tracking])
            .await?;
        Ok(Some(out.lines().map(str::to_string).collect()))
    }

    /// Merges `branch` into the branch checked out in `dir` (a fast-forward
    /// when possible, else a merge commit). On any failure the merge is
    /// aborted, so the tree is as it was, and git's error is returned.
    pub async fn merge_branch(&self, dir: &Path, branch: &str) -> Result<()> {
        if branch.starts_with('-') {
            return Err(GitError::Refused("a branch cannot start with '-'".into()));
        }
        let merged = self
            .run(dir, &["merge", "--quiet", "--no-edit", branch])
            .await;
        if merged.is_err() && self.merge_in_progress(dir).await {
            self.run(dir, &["merge", "--abort"]).await?;
        }
        merged.map(drop)
    }

    /// No staged, unstaged or untracked changes (ignored files do not count).
    pub async fn is_clean(&self, dir: &Path) -> Result<bool> {
        Ok(self.run(dir, &["status", "--porcelain"]).await?.is_empty())
    }

    /// Fast-forwards the local `branch` to `upstream` (R-002). A branch
    /// checked out in `dir` is merged with `--ff-only`; any other one is
    /// updated through `git fetch . upstream:branch`. Git itself refuses a
    /// non-fast-forward, a dirty file in the way and a branch checked out in
    /// another worktree; the error is returned and nothing changes.
    pub async fn fast_forward_branch(
        &self,
        dir: &Path,
        branch: &str,
        upstream: &str,
    ) -> Result<()> {
        if self.current_branch(dir).await.as_deref() == Some(branch) {
            self.run(dir, &["merge", "--ff-only", "--quiet", upstream])
                .await
                .map(drop)
        } else {
            let spec = format!("{upstream}:{branch}");
            self.run_env(dir, NO_PROMPT, &["fetch", "--quiet", ".", &spec])
                .await
                .map(drop)
        }
    }

    /// `git clone <url> <dest>`, run in `dest`'s parent (which must exist).
    pub async fn clone_repo(&self, url: &str, dest: &Path) -> Result<()> {
        let parent = dest.parent().unwrap_or(Path::new("/"));
        let dest = dest.to_string_lossy();
        self.run_env(parent, NO_PROMPT, &["clone", "--quiet", "--", url, &dest])
            .await
            .map(drop)
    }

    /// Fetch one commit of `url` (`git_ref`, or its `HEAD`) with no history
    /// into `dest` (created, and made a repository) and check it out
    /// detached; returns the commit's sha. A ref that is not a branch, tag or
    /// full sha the server accepts fails with git's message.
    pub async fn fetch_shallow(
        &self,
        url: &str,
        git_ref: Option<&str>,
        dest: &Path,
    ) -> Result<String> {
        let git_ref = git_ref.unwrap_or("HEAD");
        if git_ref.starts_with('-') || url.starts_with('-') {
            return Err(GitError::Refused(
                "a URL or ref cannot start with '-'".into(),
            ));
        }
        std::fs::create_dir_all(dest).map_err(|e| GitError::Refused(e.to_string()))?;
        self.run(dest, &["init", "--quiet"]).await?;
        self.run_env(
            dest,
            NO_PROMPT,
            &[
                "-c",
                "protocol.ext.allow=never",
                "fetch",
                "--quiet",
                "--depth",
                "1",
                "--no-tags",
                "--",
                url,
                git_ref,
            ],
        )
        .await?;
        self.run(dest, &["checkout", "--quiet", "--detach", "FETCH_HEAD"])
            .await?;
        self.run(dest, &["rev-parse", "HEAD"]).await
    }

    /// Unified diff of two files outside any repository (`git diff
    /// --no-index`); empty when they are equal. A missing file counts as
    /// empty, so a new or removed file shows in full.
    pub async fn diff_files(&self, a: &Path, b: &Path) -> Result<String> {
        let dir = a.parent().unwrap_or(Path::new("/"));
        let side = |p: &Path| {
            if p.exists() {
                p.to_string_lossy().into_owned()
            } else {
                "/dev/null".to_string()
            }
        };
        let (pa, pb) = (side(a), side(b));
        let cmd = Cmd::new([
            self.bin.as_str(),
            "diff",
            "--no-index",
            "--no-color",
            "--no-ext-diff",
            "--",
            &pa,
            &pb,
        ])
        .cwd(dir)
        .env("LC_ALL", "C")
        .timeout(self.timeout)
        .as_agent();
        let out = run(cmd).await?;
        if out.timed_out {
            return Err(GitError::Timeout {
                args: "diff --no-index".into(),
            });
        }
        // Exit 1 means "they differ".
        match out.code {
            Some(0 | 1) => Ok(out.stdout_str().trim_end_matches('\n').to_string()),
            _ => Err(GitError::Failed {
                args: "diff --no-index".into(),
                stderr: out.stderr.trim().to_string(),
            }),
        }
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

    /// `git worktree add <path> <branch>` for a branch that already exists
    /// (a restore, where the branch came from a bundle).
    pub async fn worktree_add_existing(
        &self,
        repo: &Path,
        path: &Path,
        branch: &str,
    ) -> Result<()> {
        let p = path.to_string_lossy();
        self.run(repo, &["worktree", "add", "--quiet", &p, branch])
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
    /// HEAD nor the working tree are skipped. A file over [`DIFF_FILE_LIMIT`]
    /// is not read: it gets a `diff --git` header and a [`TOO_LARGE`] line.
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
            // Staging a file hashes all of it (and stores it in the
            // repository): a heap dump of several GB took minutes.
            let (mut big, mut small) = (Vec::new(), Vec::new());
            for p in present {
                let len = dir
                    .join(p)
                    .symlink_metadata()
                    .ok()
                    .filter(|m| m.is_file())
                    .map_or(0, |m| m.len());
                if len > DIFF_FILE_LIMIT {
                    big.push((p, len));
                } else {
                    small.push(p);
                }
            }
            let present = small;
            let mut out = String::new();
            if !present.is_empty() {
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
                out = self.run_env(dir, &env, &args).await?;
            }
            for (p, len) in big {
                out.push_str(&format!(
                    "diff --git a/{p} b/{p}\n{TOO_LARGE}{p} is {}, over the {} limit\n",
                    human_size(len),
                    human_size(DIFF_FILE_LIMIT)
                ));
            }
            Ok(out)
        }
        .await;
        // The index file belongs to whoever ran git (the agent user, through
        // the runner), and /tmp is sticky: remove it as the same user.
        let _ = run(Cmd::new(["rm", "-f", "--", idx.as_str()]).as_agent()).await;
        res
    }

    /// A merge is under way (`MERGE_HEAD` exists).
    pub async fn merge_in_progress(&self, dir: &Path) -> bool {
        self.run(dir, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .await
            .is_ok()
    }

    /// Commit exactly `paths` (new, changed and deleted files, taken literally)
    /// with `message`, whatever else is staged; returns the new sha. Hooks run
    /// (never `--no-verify`), so a failing hook is an error with its output.
    ///
    /// During a merge git cannot commit only some paths, so the merge is
    /// committed whole, and refused (R16) while a conflict is unresolved or
    /// when the merge stages a file that is not in `paths`.
    pub async fn commit_paths(
        &self,
        dir: &Path,
        paths: &[String],
        message: &str,
    ) -> Result<String> {
        let lit = "--literal-pathspecs";
        let mut add = vec![lit, "add", "--all", "--"];
        add.extend(paths.iter().map(String::as_str));
        if self.merge_in_progress(dir).await {
            self.run(dir, &add).await?;
            let unmerged = nul_names(
                &self
                    .run(dir, &["diff", "-z", "--name-only", "--diff-filter=U"])
                    .await?,
            );
            if !unmerged.is_empty() {
                return Err(GitError::Refused(format!(
                    "unresolved conflicts in {}",
                    unmerged.join(", ")
                )));
            }
            let staged = nul_names(
                &self
                    .run(dir, &["diff", "-z", "--cached", "--name-only", "HEAD"])
                    .await?,
            );
            let extra: Vec<String> = staged.into_iter().filter(|s| !paths.contains(s)).collect();
            if !extra.is_empty() {
                return Err(GitError::Refused(format!(
                    "the merge also changes {}: propose them too",
                    extra.join(", ")
                )));
            }
            self.run(dir, &["commit", "-m", message]).await?;
            return self.rev_parse(dir, "HEAD").await;
        }
        self.run(dir, &add).await?;
        let mut commit = vec![lit, "commit", "--only", "-m", message, "--"];
        commit.extend(paths.iter().map(String::as_str));
        self.run(dir, &commit).await?;
        self.rev_parse(dir, "HEAD").await
    }

    /// Files touched by one commit; a merge commit's are those it changes
    /// against its first parent (what the merge brought into the branch).
    pub async fn commit_files(&self, dir: &Path, sha: &str) -> Result<Vec<String>> {
        let out = self
            .run(
                dir,
                &[
                    "diff-tree",
                    "-z",
                    "--no-commit-id",
                    "--name-only",
                    "-r",
                    "--root",
                    "--diff-merges=first-parent",
                    sha,
                ],
            )
            .await?;
        Ok(nul_names(&out))
    }

    /// The branch's own commits since `base`, oldest first, as `(sha, subject)`.
    /// First parent only: a merge of the base counts as one commit and the
    /// base's commits are not listed.
    pub async fn branch_commits(&self, dir: &Path, base: &str) -> Result<Vec<(String, String)>> {
        let range = format!("{base}..HEAD");
        let out = self
            .run(
                dir,
                &[
                    "log",
                    "--first-parent",
                    "--reverse",
                    "--format=%H%x00%s",
                    &range,
                    "--",
                ],
            )
            .await?;
        Ok(out
            .lines()
            .filter_map(|l| l.split_once('\0'))
            .map(|(sha, s)| (sha.to_string(), s.to_string()))
            .collect())
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

    /// Pre-push guard (R-1204): a hooks folder for this worktree only
    /// (`core.hooksPath` in the worktree's own config, which needs
    /// `extensions.worktreeConfig`). It links every hook of the folder git
    /// used before, so the repository's other hooks keep running, and writes
    /// `pre-push` as a script that asks `xherd guard pre-push` first and then
    /// runs the original `pre-push`, if any. The original folder is the last
    /// `core.hooksPath` of a scope other than the worktree's (our own setting
    /// is never mistaken for it), else the repository's `hooks`. Safe to call
    /// again (R8): the script is rewritten only when its text changes (a new
    /// `xherd` path), and the answer says whether anything was written.
    /// Hooks can be skipped (`--no-verify`): this stops mistakes, not a
    /// determined agent.
    pub async fn install_push_guard(
        &self,
        worktree: &Path,
        hooks_dir: &Path,
        xherd_bin: &str,
        branch: &str,
    ) -> Result<bool> {
        let fail = |e: std::io::Error| GitError::Failed {
            args: "push guard".into(),
            stderr: format!("{}: {e}", hooks_dir.display()),
        };
        let configured = self
            .run(
                worktree,
                &[
                    "config",
                    "--type=path",
                    "--show-scope",
                    "--get-all",
                    "core.hooksPath",
                ],
            )
            .await
            .unwrap_or_default() // exit 1: the key is not set
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .rfind(|(scope, _)| *scope != "worktree")
            .map(|(_, v)| v.to_string());
        let base = match configured {
            Some(v) if Path::new(&v).is_absolute() => PathBuf::from(v),
            Some(v) => worktree.join(v),
            None => self.common_dir(worktree).await?.join("hooks"),
        };
        let base = base.canonicalize().unwrap_or(base);
        let mut wrote = !hooks_dir.is_dir();
        std::fs::create_dir_all(hooks_dir).map_err(fail)?;
        for entry in std::fs::read_dir(&base).into_iter().flatten().flatten() {
            let name = entry.file_name();
            let n = name.to_string_lossy();
            let link = hooks_dir.join(&name);
            if n == "pre-push" || n.ends_with(".sample") || link.symlink_metadata().is_ok() {
                continue;
            }
            std::os::unix::fs::symlink(entry.path(), link).map_err(fail)?;
            wrote = true;
        }
        let q = sh_quote;
        let script = format!(
            "#!/bin/sh\n# xherd push guard (R-1204)\ninput=$(cat)\n\
             printf '%s\\n' \"$input\" | {} guard pre-push --branch {} \"$@\" || exit 1\n\
             if [ -x {orig} ]; then printf '%s\\n' \"$input\" | {orig} \"$@\"; fi\n",
            q(xherd_bin),
            q(branch),
            orig = q(&base.join("pre-push").to_string_lossy()),
        );
        let hook = hooks_dir.join("pre-push");
        if std::fs::read_to_string(&hook).ok().as_deref() != Some(script.as_str()) {
            xherd_core::paths::write_atomic(&hook, script.as_bytes(), Some(0o755)).map_err(fail)?;
            wrote = true;
        }
        self.run(worktree, &["config", "extensions.worktreeConfig", "true"])
            .await?;
        self.run(
            worktree,
            &[
                "config",
                "--worktree",
                "core.hooksPath",
                &hooks_dir.to_string_lossy(),
            ],
        )
        .await?;
        Ok(wrote)
    }

    /// Append patterns to `.git/info/exclude` of the repo `dir` belongs to,
    /// skipping ones already present (R-005b).
    pub async fn add_excludes(&self, dir: &Path, patterns: &[&str]) -> Result<()> {
        let file = self.common_dir(dir).await?.join("info").join("exclude");
        let existing = xherd_core::runner::read_file(&file)
            .await
            .ok()
            .flatten()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
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
        xherd_core::runner::write_file(&file, content.as_bytes(), None)
            .await
            .map_err(|e| GitError::Failed {
                args: "info/exclude".into(),
                stderr: e.to_string(),
            })
    }
}

/// Path names from a `-z` listing, unquoted as git stores them.
fn nul_names(out: &str) -> Vec<String> {
    out.split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
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

/// Single-quote a string for `sh`.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
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

    #[tokio::test]
    async fn run_raw_keeps_trailing_newlines_that_run_trims() {
        let dir = tempfile::tempdir().unwrap();
        let git = Git::new("git");
        let d = dir.path();
        git.run(d, &["init", "-q"]).await.unwrap();
        std::fs::write(d.join("f.txt"), "a\n\n\n").unwrap();
        git.run(d, &["add", "f.txt"]).await.unwrap();
        git.run(
            d,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "-m",
                "m",
            ],
        )
        .await
        .unwrap();
        assert_eq!(git.run(d, &["show", "HEAD:f.txt"]).await.unwrap(), "a");
        assert_eq!(
            git.run_raw(d, &["show", "HEAD:f.txt"]).await.unwrap(),
            "a\n\n\n"
        );
    }

    /// R8: a second install with another binary rewrites the guard, keeps
    /// chaining the repository's own `pre-push`, and a third changes nothing.
    #[tokio::test]
    async fn push_guard_is_rewritten_for_a_new_binary_and_keeps_the_original_hook() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let repo = root.join("repo");
        let wt = root.join("wt");
        std::fs::create_dir_all(&repo).unwrap();
        let git = Git::new("git");
        git.run(&repo, &["init", "-q", "-b", "main"]).await.unwrap();
        git.run(
            &repo,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "m",
            ],
        )
        .await
        .unwrap();
        let original = repo.join(".git/hooks/pre-push");
        std::fs::write(&original, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::write(repo.join(".git/hooks/commit-msg"), "#!/bin/sh\nexit 0\n").unwrap();
        git.run(
            &repo,
            &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
        )
        .await
        .unwrap();
        let hooks = wt.join(".xherd/hooks");
        let hook = hooks.join("pre-push");

        assert!(
            git.install_push_guard(&wt, &hooks, "/bin/xherd-a", "feat")
                .await
                .unwrap()
        );
        let first = std::fs::read_to_string(&hook).unwrap();
        assert!(first.contains("'/bin/xherd-a'"), "{first}");

        // Same inputs: nothing is written.
        assert!(
            !git.install_push_guard(&wt, &hooks, "/bin/xherd-a", "feat")
                .await
                .unwrap()
        );

        // A new binary: rewritten, and the original hook is still the one chained
        // (not our own script, which is what the worktree's hooksPath now names).
        assert!(
            git.install_push_guard(&wt, &hooks, "/bin/xherd-b", "feat")
                .await
                .unwrap()
        );
        let second = std::fs::read_to_string(&hook).unwrap();
        assert!(second.contains("'/bin/xherd-b'"), "{second}");
        assert!(!second.contains("xherd-a"), "{second}");
        assert!(
            second.contains(&format!("[ -x '{}' ]", original.display())),
            "{second}"
        );
        assert!(hooks.join("commit-msg").symlink_metadata().is_ok());
    }

    #[tokio::test]
    async fn unpushed_lists_local_only_commits_and_push_sends_them() {
        let d = tempfile::tempdir().unwrap();
        let g = Git::new("git");
        let bare = d.path().join("bare.git");
        std::fs::create_dir(&bare).unwrap();
        g.run(&bare, &["init", "--quiet", "--bare", "-b", "main"])
            .await
            .unwrap();
        let repo = d.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        g.init(&repo, "main").await.unwrap();
        for args in [
            &["config", "user.email", "t@example.com"][..],
            &["config", "user.name", "t"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.run(&repo, args).await.unwrap();
        }
        let one = g.commit_empty(&repo, "one").await.unwrap();
        g.remote_add(&repo, "origin", &bare.to_string_lossy())
            .await
            .unwrap();

        assert_eq!(g.unpushed(&repo, "main").await.unwrap(), None);
        g.push(&repo, "origin", "main").await.unwrap();
        assert_eq!(g.rev_parse(&bare, "main").await.unwrap(), one);
        assert_eq!(g.unpushed(&repo, "main").await.unwrap(), Some(vec![]));

        let two = g.commit_empty(&repo, "two").await.unwrap();
        assert_eq!(
            g.unpushed(&repo, "main").await.unwrap(),
            Some(vec![two.clone()])
        );
        g.push(&repo, "origin", "main").await.unwrap();
        assert_eq!(g.rev_parse(&bare, "main").await.unwrap(), two);
        assert_eq!(g.unpushed(&repo, "main").await.unwrap(), Some(vec![]));

        assert!(g.push(&repo, "origin", "-x").await.is_err());
        assert!(g.unpushed(&repo, "-x").await.is_err());
    }
}
