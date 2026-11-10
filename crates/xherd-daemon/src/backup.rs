//! Backups and restore (spec §15, R-1401…R-1405, design §10.4).
//!
//! One `tar.zst` archive holds: a consistent database copy, `config.json`
//! (without the user token), plugins, plugin settings and `archive/`; for each
//! agent that is not finished its `.xherd/` folder, a git bundle of the branch
//! (commits beyond the agent's base), a patch of uncommitted changes and the
//! untracked files. A `manifest.json` carries the sha256 of every part.
//!
//! Not included, because they can be recreated or re-entered: `secrets.json`,
//! runtime logins, toolchains and caches, repository clones, and the worktree
//! files xherd writes itself (`.xherd/agent.json`, `instructions.md`, the
//! pre-push guard, the toolchain plan). The daemon finishes a restored
//! worktree at its next start (`Daemon::repair_restored`).
//!
//! `restore` runs with the daemon stopped and can move everything to another
//! machine: `--workspace` / `--map OLD=NEW` rewrite the stored paths.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use xherd_core::paths::{Paths, write_atomic};
use xherd_core::{Agent, Millis, Status};
use xherd_git::Git;
use xherd_store::Store;

use crate::Daemon;

pub type Result<T> = std::result::Result<T, String>;

const FORMAT: u32 = 1;
/// Updates (R-1405) keep this many backups of their own.
pub const UPDATE_KEEP: usize = 3;
/// Untracked files larger than this are left out (with a warning).
const MAX_UNTRACKED: u64 = 20 * 1024 * 1024;
/// Parts of `.xherd/` that xherd writes again by itself.
const XHERD_SKIP: &[&str] = &[
    "agent.json",
    "instructions.md",
    "hooks",
    "toolchain.json",
    "toolchain.resolved.json",
    "backup.bundle",
    "backup.patch",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: u32,
    pub xherd_version: String,
    pub created_at: Millis,
    /// The workspace root when the backup was taken (the default for `--workspace`).
    pub workspace: String,
    /// sha256 of every file of the archive but this one, by relative path.
    pub files: BTreeMap<String, String>,
    pub agents: Vec<AgentEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEntry {
    pub id: String,
    pub name: String,
    pub branch: Option<String>,
    /// `origin`'s URL: a restore on another machine clones it when the repo is missing.
    pub remote: Option<String>,
    pub bundle: bool,
    /// The branch had commits but the bundle could not be made.
    #[serde(default)]
    pub bundle_failed: bool,
    pub patch: bool,
}

#[derive(Debug)]
pub struct Created {
    pub path: PathBuf,
    pub agents: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Restored {
    pub agents: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RestoreOptions {
    /// `OLD=NEW` prefixes applied to the agents' stored paths.
    pub maps: Vec<(String, String)>,
    /// New workspace root: also saved in `config.json`, and maps the old one.
    pub workspace: Option<String>,
    /// Replace an existing database (kept in `backups/`).
    pub force: bool,
}

fn io<T>(what: impl std::fmt::Display, r: std::io::Result<T>) -> Result<T> {
    r.map_err(|e| format!("{what}: {e}"))
}

fn sha_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Regular files below `dir`, as sorted paths relative to it (symlinks are not followed or listed).
fn walk(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    fn go(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let t = e.file_type()?;
            if t.is_dir() {
                go(root, &e.path(), out)?;
            } else if t.is_file() {
                out.push(
                    e.path()
                        .strip_prefix(root)
                        .expect("below root")
                        .to_path_buf(),
                );
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    if dir.is_dir() {
        go(dir, dir, &mut out)?;
    }
    out.sort();
    Ok(out)
}

/// Copy the regular files below `src` into `dst`; top-level names in `skip`
/// are left out. A missing `src` copies nothing. `keep_existing`: never
/// overwrite a file of `dst`.
fn copy_tree(src: &Path, dst: &Path, skip: &[&str], keep_existing: bool) -> std::io::Result<()> {
    for rel in walk(src)? {
        let top = rel
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned());
        if top.is_some_and(|t| skip.contains(&t.as_str())) {
            continue;
        }
        let to = dst.join(&rel);
        if keep_existing && to.exists() {
            continue;
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src.join(&rel), &to)?;
    }
    Ok(())
}

/// `p` with the first matching `OLD=NEW` prefix replaced (whole path components only).
pub fn remap(p: &str, maps: &[(String, String)]) -> String {
    for (old, new) in maps {
        let old = old.trim_end_matches('/');
        let new = new.trim_end_matches('/');
        if p == old {
            return new.to_string();
        }
        if let Some(rest) = p.strip_prefix(old).filter(|r| r.starts_with('/')) {
            return format!("{new}{rest}");
        }
    }
    p.to_string()
}

fn is_live(s: Status) -> bool {
    s.is_live() || s == Status::Queued
}

// ---- create -----------------------------------------------------------

/// Write a backup of everything described in the module docs into `dest`.
pub async fn create(
    paths: &Paths,
    workspace: &Path,
    store: &Store,
    git: &Git,
    dest: &Path,
    label: &str,
    now: Millis,
) -> Result<Created> {
    io(dest.display(), std::fs::create_dir_all(dest))?;
    let stage = dest.join(format!(".staging-{}-{now}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    let out = fill_stage(paths, workspace, store, git, &stage, now).await;
    let res = match out {
        Ok((manifest, warnings)) => pack(&stage, dest, label, now / 1000).map(|path| Created {
            path,
            agents: manifest.agents.len(),
            warnings,
        }),
        Err(e) => Err(e),
    };
    let _ = std::fs::remove_dir_all(&stage);
    res
}

async fn fill_stage(
    paths: &Paths,
    workspace: &Path,
    store: &Store,
    git: &Git,
    stage: &Path,
    now: Millis,
) -> Result<(Manifest, Vec<String>)> {
    let mut warnings = Vec::new();
    io(stage.display(), std::fs::create_dir_all(stage.join("db")))?;
    store
        .snapshot(&stage.join("db/xherd.db"))
        .await
        .map_err(|e| format!("database copy: {e}"))?;

    // config.json without the user token (the daemon makes a new one).
    let data = stage.join("data");
    io(data.display(), std::fs::create_dir_all(&data))?;
    if let Ok(text) = std::fs::read_to_string(paths.config()) {
        let mut v: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("config.json: {e}"))?;
        if let Some(o) = v.as_object_mut() {
            o.remove("userToken");
        }
        let bytes = serde_json::to_vec_pretty(&v).expect("json");
        io(
            "config.json",
            write_atomic(&data.join("config.json"), &bytes, Some(0o600)),
        )?;
    }
    for d in ["plugins", "archive", "plugin-config"] {
        io(d, copy_tree(&paths.root.join(d), &data.join(d), &[], false))?;
    }

    let mut entries = Vec::new();
    let agents = store.list_agents().await.map_err(|e| e.to_string())?;
    for a in agents
        .iter()
        .filter(|a| !matches!(a.status, Status::Done | Status::Cancelled))
    {
        match backup_agent(git, a, stage, &mut warnings).await {
            Ok(Some(e)) => entries.push(e),
            Ok(None) => {}
            Err(e) => warnings.push(format!("{}: {e}", a.name)),
        }
    }

    let mut files = BTreeMap::new();
    for rel in io("staging", walk(stage))? {
        files.insert(
            rel.to_string_lossy().into_owned(),
            io(rel.display(), sha_file(&stage.join(&rel)))?,
        );
    }
    let manifest = Manifest {
        format: FORMAT,
        xherd_version: xherd_core::VERSION.to_string(),
        created_at: now,
        workspace: workspace.to_string_lossy().into_owned(),
        files,
        agents: entries,
    };
    let bytes = serde_json::to_vec_pretty(&manifest).expect("json");
    io(
        "manifest.json",
        std::fs::write(stage.join("manifest.json"), bytes),
    )?;
    Ok((manifest, warnings))
}

async fn backup_agent(
    git: &Git,
    a: &Agent,
    stage: &Path,
    warnings: &mut Vec<String>,
) -> Result<Option<AgentEntry>> {
    let Some(wt) = a.workdir.as_deref().map(PathBuf::from) else {
        return Ok(None);
    };
    if !wt.is_dir() {
        return Err(format!(
            "worktree {} is missing; only its database record is kept",
            wt.display()
        ));
    }
    let adir = stage.join("agents").join(&a.name);
    io("agent folder", std::fs::create_dir_all(&adir))?;
    io(
        ".xherd",
        copy_tree(&wt.join(".xherd"), &adir.join("xherd"), XHERD_SKIP, false),
    )?;

    let mut entry = AgentEntry {
        id: a.id.clone(),
        name: a.name.clone(),
        branch: None,
        remote: git.run(&wt, &["remote", "get-url", "origin"]).await.ok(),
        bundle: false,
        bundle_failed: false,
        patch: false,
    };
    let Some(branch) = git.current_branch(&wt).await else {
        warnings.push(format!(
            "{}: no branch checked out; git state not saved",
            a.name
        ));
        return Ok(Some(entry));
    };
    entry.branch = Some(branch.clone());

    // Commits beyond the base, or the whole branch when the base is unknown.
    let base = match a.base.as_deref() {
        Some(b) if git.rev_parse(&wt, b).await.is_ok() => Some(b),
        _ => None,
    };
    let ahead = match base {
        Some(b) => git.count_commits(&wt, &branch, b).await.unwrap_or(1),
        None => 1,
    };
    // The agent user writes these two (git runs as the agent, R-1203); the
    // shared `.xherd/` is where both users can.
    let tmp = wt.join(".xherd/backup.bundle");
    let patch = wt.join(".xherd/backup.patch");
    if ahead > 0 {
        let _ = std::fs::remove_file(&tmp);
        // A failed bundle must not cost the agent's other parts: say so, here
        // and again at restore (`bundle_failed`), and go on.
        let made = match git.bundle_create(&wt, &tmp, &branch, base).await {
            Ok(()) => std::fs::copy(&tmp, adir.join("branch.bundle"))
                .map(drop)
                .map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        let _ = std::fs::remove_file(&tmp);
        match made {
            Ok(()) => entry.bundle = true,
            Err(e) => {
                entry.bundle_failed = true;
                warnings.push(format!("{}: its commits were not saved: {e}", a.name));
            }
        }
    }

    let _ = std::fs::remove_file(&patch);
    let arg = format!("--output={}", patch.display());
    match git
        .run(&wt, &["diff", "--binary", "--no-ext-diff", &arg, "HEAD"])
        .await
    {
        Ok(_) => {
            if std::fs::metadata(&patch).is_ok_and(|m| m.len() > 0) {
                io("patch", std::fs::copy(&patch, adir.join("wip.patch")))?;
                entry.patch = true;
            }
        }
        Err(e) => warnings.push(format!("{}: uncommitted changes not saved: {e}", a.name)),
    }
    let _ = std::fs::remove_file(&patch);

    let untracked = git
        .run(&wt, &["ls-files", "--others", "--exclude-standard"])
        .await
        .unwrap_or_default();
    for rel in untracked.lines().filter(|l| !l.is_empty()) {
        let from = wt.join(rel);
        let Ok(meta) = std::fs::symlink_metadata(&from) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        if meta.len() > MAX_UNTRACKED {
            warnings.push(format!("{}: {rel} is over 20 MiB and was left out", a.name));
            continue;
        }
        let to = adir.join("untracked").join(rel);
        if let Some(p) = to.parent() {
            io("untracked", std::fs::create_dir_all(p))?;
        }
        io(rel, std::fs::copy(&from, &to))?;
    }
    Ok(Some(entry))
}

fn pack(stage: &Path, dest: &Path, label: &str, secs: i64) -> Result<PathBuf> {
    let name = format!("xherd-{label}-{secs}.tar.zst");
    let part = dest.join(format!(".{name}.part"));
    let file = io(part.display(), std::fs::File::create(&part))?;
    let enc = io("zstd", zstd::Encoder::new(file, 3))?;
    let mut b = tar::Builder::new(enc);
    b.follow_symlinks(false);
    io("tar", b.append_dir_all(".", stage))?;
    let enc = io("tar", b.into_inner())?;
    let mut file = io("zstd", enc.finish())?;
    io("archive", file.flush())?;
    io("archive", file.sync_all())?;
    let path = dest.join(&name);
    io(path.display(), std::fs::rename(&part, &path))?;
    Ok(path)
}

/// Archives of `dir`, newest first.
pub fn list(dir: &Path) -> Vec<(String, u64, Millis)> {
    let mut v: Vec<(String, u64, Millis)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with("xherd-") || !name.ends_with(".tar.zst") {
                return None;
            }
            let m = e.metadata().ok()?;
            let at = m
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_millis() as Millis;
            Some((name, m.len(), at))
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v
}

/// Delete all but the newest `keep` archives made with `label`.
pub fn prune(dir: &Path, label: &str, keep: usize) {
    let prefix = format!("xherd-{label}-");
    for (name, _, _) in list(dir)
        .into_iter()
        .filter(|(n, _, _)| n.starts_with(&prefix))
        .skip(keep)
    {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

// ---- verify -----------------------------------------------------------

/// Unpack into `work` (which must not exist) and check it (R-1404): every
/// part's hash, the database, and that each bundle is readable.
async fn open(archive: &Path, work: &Path, git: &Git) -> Result<Manifest> {
    let file = io(archive.display(), std::fs::File::open(archive))?;
    let dec = io("zstd", zstd::Decoder::new(file))?;
    io(work.display(), std::fs::create_dir_all(work))?;
    io("unpack", tar::Archive::new(dec).unpack(work))?;

    let text = io(
        "manifest.json",
        std::fs::read_to_string(work.join("manifest.json")),
    )?;
    let m: Manifest = serde_json::from_str(&text).map_err(|e| format!("manifest.json: {e}"))?;
    if m.format != FORMAT {
        return Err(format!(
            "backup format {} is not supported (this xherd reads {FORMAT})",
            m.format
        ));
    }
    for (rel, sha) in &m.files {
        let got = io(rel, sha_file(&work.join(rel)))?;
        if &got != sha {
            return Err(format!("{rel}: hash does not match the manifest"));
        }
    }
    xherd_store::check_integrity(&work.join("db/xherd.db")).map_err(|e| e.to_string())?;
    for e in m.agents.iter().filter(|e| e.bundle) {
        let b = work.join("agents").join(&e.name).join("branch.bundle");
        git.run(work, &["bundle", "list-heads", &b.to_string_lossy()])
            .await
            .map_err(|err| format!("{}: bundle: {err}", e.name))?;
    }
    Ok(m)
}

/// Check an archive without restoring it; `scratch` is where it is unpacked.
pub async fn verify(archive: &Path, scratch: &Path, git: &Git, now: Millis) -> Result<Manifest> {
    let work = scratch.join(format!("verify-{now}"));
    let r = open(archive, &work, git).await;
    let _ = std::fs::remove_dir_all(&work);
    r
}

// ---- restore ----------------------------------------------------------

/// Restore into `paths` (the daemon must not be running). Agents come back
/// `paused`; a worktree that is missing is recreated from the repository, the
/// branch bundle and the patch (cloning the repository from its remote when
/// it is not there); one that exists is only completed, never overwritten.
pub async fn restore(
    paths: &Paths,
    git: &Git,
    archive: &Path,
    opts: &RestoreOptions,
    now: Millis,
) -> Result<Restored> {
    if paths.db().exists() && !opts.force {
        return Err(format!(
            "{} already has a database; --force keeps it in backups/ and restores over it",
            paths.root.display()
        ));
    }
    io(paths.root.display(), paths.ensure())?;
    let work = paths.scratch().join(format!("restore-{now}"));
    let res = restore_from(paths, git, archive, opts, now, &work).await;
    let _ = std::fs::remove_dir_all(&work);
    res
}

async fn restore_from(
    paths: &Paths,
    git: &Git,
    archive: &Path,
    opts: &RestoreOptions,
    now: Millis,
    work: &Path,
) -> Result<Restored> {
    let m = open(archive, work, git).await?;
    let mut out = Restored::default();

    let mut maps = opts.maps.clone();
    if let Some(ws) = &opts.workspace {
        maps.push((m.workspace.clone(), ws.clone()));
    }

    // The old database stays reachable.
    for ext in ["", "-wal", "-shm"] {
        let from = PathBuf::from(format!("{}{ext}", paths.db().display()));
        if from.exists() {
            let to = paths.backups().join(format!("pre-restore-{now}.db{ext}"));
            io(from.display(), std::fs::rename(&from, &to))?;
        }
    }
    io(
        "database",
        std::fs::copy(work.join("db/xherd.db"), paths.db()),
    )?;
    for d in ["plugins", "archive", "plugin-config"] {
        io(
            d,
            copy_tree(&work.join("data").join(d), &paths.root.join(d), &[], false),
        )?;
    }
    if let Ok(text) = std::fs::read_to_string(work.join("data/config.json")) {
        let mut v: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("config.json: {e}"))?;
        if let (Some(ws), Some(o)) = (&opts.workspace, v.as_object_mut()) {
            o.insert("workspace".into(), ws.clone().into());
        }
        let bytes = serde_json::to_vec_pretty(&v).expect("json");
        io(
            "config.json",
            write_atomic(&paths.config(), &bytes, Some(0o600)),
        )?;
    }

    let store = Store::open(&paths.db(), &paths.backups()).map_err(|e| e.to_string())?;
    for e in &m.agents {
        let Some(mut a) = store.get_agent(&e.id).await.map_err(|e| e.to_string())? else {
            out.warnings
                .push(format!("{}: not in the database", e.name));
            continue;
        };
        a.workdir = a.workdir.map(|p| remap(&p, &maps));
        a.repo = a.repo.map(|p| remap(&p, &maps));
        if is_live(a.status) {
            a.status = Status::Paused;
            a.status_since = now;
        }
        a.pause_requested_at = None;
        store.put_agent(&a).await.map_err(|e| e.to_string())?;
        if let Err(w) = restore_agent(
            git,
            e,
            &a,
            &work.join("agents").join(&e.name),
            &mut out.warnings,
        )
        .await
        {
            out.warnings.push(format!("{}: {w}", e.name));
        }
        out.agents.push(e.name.clone());
    }
    Ok(out)
}

async fn restore_agent(
    git: &Git,
    e: &AgentEntry,
    a: &Agent,
    adir: &Path,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let (Some(wt), Some(repo)) = (
        a.workdir.as_deref().map(PathBuf::from),
        a.repo.as_deref().map(PathBuf::from),
    ) else {
        return Err("no worktree recorded".into());
    };
    if wt.is_dir() {
        io(
            ".xherd",
            copy_tree(&adir.join("xherd"), &wt.join(".xherd"), &[], true),
        )?;
        return Err(format!(
            "{} exists and was kept as it is (only missing .xherd files added)",
            wt.display()
        ));
    }
    let branch = e
        .branch
        .as_deref()
        .ok_or("no branch was saved; the worktree is not recreated")?;
    if !git.is_repo(&repo).await {
        let url = e.remote.as_deref().ok_or_else(|| {
            format!("repository {} is missing and had no remote; restore it there, then run the restore again", repo.display())
        })?;
        if let Some(p) = repo.parent() {
            io(p.display(), std::fs::create_dir_all(p))?;
        }
        git.clone_repo(url, &repo)
            .await
            .map_err(|err| format!("clone {url}: {err}"))?;
    }
    if !git.branch_exists(&repo, branch).await {
        if e.bundle {
            let b = adir.join("branch.bundle");
            let spec = format!("{branch}:{branch}");
            git.run(&repo, &["fetch", "--quiet", &b.to_string_lossy(), &spec])
                .await
                .map_err(|err| format!("branch {branch} from its bundle: {err}"))?;
        } else {
            if e.bundle_failed {
                warnings.push(format!(
                    "{}: its commits were not in the backup; the branch starts again from its base",
                    e.name
                ));
            }
            let base = a
                .base
                .as_deref()
                .ok_or("no base to recreate the branch from")?;
            git.run(&repo, &["branch", "--no-track", branch, base])
                .await
                .map_err(|err| format!("branch {branch} from {base}: {err}"))?;
        }
    }
    if let Some(p) = wt.parent() {
        io(p.display(), std::fs::create_dir_all(p))?;
    }
    git.worktree_add_existing(&repo, &wt, branch)
        .await
        .map_err(|err| err.to_string())?;
    git.add_excludes(&wt, &[".xherd/"])
        .await
        .map_err(|err| err.to_string())?;
    io(
        ".xherd",
        copy_tree(&adir.join("xherd"), &wt.join(".xherd"), &[], false),
    )?;
    // Present so that the next daemon start finishes this worktree.
    io(".xherd", std::fs::create_dir_all(wt.join(".xherd")))?;
    if e.patch {
        let p = adir.join("wip.patch");
        git.run(&wt, &["apply", "--binary", &p.to_string_lossy()])
            .await
            .map_err(|err| format!("uncommitted changes did not apply: {err}"))?;
    }
    io(
        "untracked",
        copy_tree(&adir.join("untracked"), &wt, &[], true),
    )?;
    Ok(())
}

/// Where backups go by default: `<data>/backups`, and in the container the
/// workspace's `xherd-backups` (the volume is not visible on the host).
pub fn default_dir(paths: &Paths, config: &xherd_core::config::Config, sandbox: bool) -> PathBuf {
    if sandbox {
        config.workspace_root().join("xherd-backups")
    } else {
        paths.backups()
    }
}

// ---- daemon side ------------------------------------------------------

impl Daemon {
    pub fn backup_dir(&self) -> PathBuf {
        default_dir(&self.paths, &self.config, self.sandbox)
    }

    /// `POST /backups`. `label` `update` is pruned to the last `UPDATE_KEEP` (R-1405).
    pub async fn backup_create(&self, label: &str) -> Result<Created> {
        let dir = self.backup_dir();
        let c = create(
            &self.paths,
            &self.config.workspace_root(),
            &self.store,
            &xherd_git::Git::new("git"),
            &dir,
            label,
            self.clock.now(),
        )
        .await?;
        if label == "update" {
            prune(&dir, label, UPDATE_KEEP);
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remap_replaces_whole_components_only() {
        let m = vec![("/old/ws".to_string(), "/new/ws/".to_string())];
        assert_eq!(remap("/old/ws", &m), "/new/ws");
        assert_eq!(remap("/old/ws/worktrees/r/a", &m), "/new/ws/worktrees/r/a");
        assert_eq!(remap("/old/ws2/x", &m), "/old/ws2/x");
        assert_eq!(remap("/elsewhere", &m), "/elsewhere");
    }
}
