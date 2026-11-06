//! Data root and well-known files (design §3.3).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    /// `XHERD_HOME`, else `$XDG_DATA_HOME/xherd`, else `~/.local/share/xherd`.
    pub fn from_env() -> Paths {
        Self::resolve(
            std::env::var_os("XHERD_HOME").map(PathBuf::from),
            std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
            std::env::var_os("HOME").map(PathBuf::from),
        )
    }

    pub fn resolve(
        xherd_home: Option<PathBuf>,
        xdg: Option<PathBuf>,
        home: Option<PathBuf>,
    ) -> Paths {
        let nonempty = |p: Option<PathBuf>| p.filter(|p| !p.as_os_str().is_empty());
        let root = nonempty(xherd_home)
            .or_else(|| nonempty(xdg).map(|x| x.join("xherd")))
            .unwrap_or_else(|| {
                nonempty(home)
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".local/share/xherd")
            });
        Paths { root }
    }

    pub fn at(root: impl AsRef<Path>) -> Paths {
        Paths {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn config(&self) -> PathBuf {
        self.root.join("config.json")
    }
    pub fn secrets(&self) -> PathBuf {
        self.root.join("secrets.json")
    }
    pub fn db(&self) -> PathBuf {
        self.root.join("xherd.db")
    }
    pub fn hook_queue(&self) -> PathBuf {
        self.root.join("hook-queue.jsonl")
    }
    pub fn archive(&self) -> PathBuf {
        self.root.join("archive")
    }
    pub fn plugins(&self) -> PathBuf {
        self.root.join("plugins")
    }
    pub fn scratch(&self) -> PathBuf {
        self.root.join("scratch")
    }
    pub fn backups(&self) -> PathBuf {
        self.root.join("backups")
    }
    pub fn plugin_config(&self, name: &str) -> PathBuf {
        self.root.join("plugin-config").join(name)
    }
    pub fn plugin_state(&self, name: &str) -> PathBuf {
        self.root.join("plugin-state").join(name)
    }

    /// Create the root (0700) and its subfolders.
    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))?;
        }
        for d in [
            self.archive(),
            self.plugins(),
            self.scratch(),
            self.backups(),
        ] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }
}

/// Write a file atomically: temp file in the same folder, fsync, rename.
pub fn write_atomic(path: &Path, bytes: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    {
        let mut f = std::fs::File::create(&tmp)?;
        #[cfg(unix)]
        if let Some(m) = mode {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(m))?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_order() {
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(
            Paths::resolve(p("/x"), p("/d"), p("/h")).root,
            PathBuf::from("/x")
        );
        assert_eq!(
            Paths::resolve(None, p("/d"), p("/h")).root,
            PathBuf::from("/d/xherd")
        );
        assert_eq!(
            Paths::resolve(p(""), None, p("/h")).root,
            PathBuf::from("/h/.local/share/xherd")
        );
    }

    #[test]
    fn atomic_write_with_mode() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("sub/secrets.json");
        write_atomic(&f, b"{}", Some(0o600)).unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"{}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&f).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
