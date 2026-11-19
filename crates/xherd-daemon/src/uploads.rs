//! Files the user drops or pastes into a job's terminal (R1–R6): they are
//! stored under `<workdir>/.xherd/uploads/` through the runner (so they belong
//! to the agents' user), within three live limits, and read back by path.

use std::path::{Path, PathBuf};

use crate::{ActionError, Daemon, UserAction, workdir_of};

fn fail(code: &'static str, message: impl Into<String>) -> ActionError {
    ActionError {
        code,
        message: message.into(),
    }
}

fn io(e: std::io::Error) -> ActionError {
    fail("io", e.to_string())
}

fn mib(n: u64) -> String {
    format!("{} MiB", n / (1024 * 1024))
}

/// One upload request: holds the daemon-wide upload lock, the limits read when
/// it began, and the files it made so far (removed again by `abort`).
pub struct Upload {
    _lock: tokio::sync::OwnedMutexGuard<()>,
    dir: PathBuf,
    max_bytes: u64,
    max_job_bytes: u64,
    max_files: usize,
    bytes: u64,
    files: usize,
    created: Vec<PathBuf>,
}

impl Upload {
    /// The largest file this upload takes (`uploadMaxBytes`).
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// The most bytes the job's uploads may hold in all (`uploadMaxJobBytes`).
    pub fn max_job_bytes(&self) -> u64 {
        self.max_job_bytes
    }

    /// Store one file. `name` is the client's name, cleaned here; a taken name
    /// gets a number. Over a limit is `too_large`, and nothing is written.
    pub async fn add(&mut self, name: &str, bytes: &[u8]) -> Result<PathBuf, ActionError> {
        let len = bytes.len() as u64;
        if len > self.max_bytes {
            return Err(fail(
                "too_large",
                format!("a file is larger than {}", mib(self.max_bytes)),
            ));
        }
        if self.files + 1 > self.max_files {
            return Err(fail(
                "too_large",
                format!("a job takes at most {} uploaded files", self.max_files),
            ));
        }
        if self.bytes + len > self.max_job_bytes {
            return Err(fail(
                "too_large",
                format!(
                    "a job's uploads may hold at most {} in all",
                    mib(self.max_job_bytes)
                ),
            ));
        }
        let clean = xherd_core::upload::clean_name(name);
        let path = xherd_core::runner::create_file(&self.dir, &clean, bytes, 0o664)
            .await
            .map_err(io)?;
        self.created.push(path.clone());
        self.bytes += len;
        self.files += 1;
        Ok(path)
    }

    /// Done: the paths stored, in the order of `add`.
    pub fn finish(self) -> Vec<PathBuf> {
        self.created
    }

    /// Refused midway: remove what this request stored (R3).
    pub async fn abort(self) {
        for p in &self.created {
            if let Err(e) = xherd_core::runner::remove_file(p, &self.dir).await {
                tracing::warn!(path = %p.display(), error = %e, "upload rollback failed");
            }
        }
    }
}

impl Daemon {
    /// Start an upload for job `who`: the job must have a worktree (R5), the
    /// limits are read now (R3a) and the existing uploads are counted.
    pub async fn begin_upload(&self, who: &str) -> Result<Upload, ActionError> {
        let a = self.action(who, UserAction::Get).await?;
        let wd = workdir_of(&a)?.to_path_buf();
        let lock = self.uploads.clone().lock_owned().await;
        let c = self.live_config();
        let dir = wd.join(".xherd").join("uploads");
        match std::fs::symlink_metadata(&dir) {
            Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
                return Err(fail("bad_path", ".xherd/uploads is not a folder"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                xherd_core::runner::mkdir(&dir, 0o2775).await.map_err(io)?;
            }
            Err(e) => return Err(io(e)),
        }
        let (mut bytes, mut files) = (0u64, 0usize);
        for e in std::fs::read_dir(&dir).map_err(io)? {
            let m = e.map_err(io)?.metadata().map_err(io)?;
            if m.is_file() {
                bytes += m.len();
                files += 1;
            }
        }
        Ok(Upload {
            _lock: lock,
            dir,
            max_bytes: c.upload_max_bytes,
            max_job_bytes: c.upload_max_job_bytes,
            max_files: c.upload_max_files,
            bytes,
            files,
            created: Vec::new(),
        })
    }

    /// The canonical path of a regular file under job `who`'s workdir, for a
    /// download (R6). `path` is absolute or relative to the workdir.
    pub async fn download_path(&self, who: &str, path: &str) -> Result<PathBuf, ActionError> {
        let a = self.action(who, UserAction::Get).await?;
        let wd = workdir_of(&a)?;
        let root = wd
            .canonicalize()
            .map_err(|_| fail("no_workspace", format!("{} has no worktree", a.name)))?;
        if path.is_empty() || path.contains('\0') {
            return Err(fail("bad_path", "the path is empty or not valid"));
        }
        let p = Path::new(path);
        let full = if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        };
        let real = full
            .canonicalize()
            .map_err(|_| fail("no_such_file", format!("{path} does not exist")))?;
        if !real.starts_with(&root) {
            return Err(fail("bad_path", format!("{path} is outside the worktree")));
        }
        if !real.is_file() {
            return Err(fail("no_such_file", format!("{path} is not a file")));
        }
        Ok(real)
    }
}
