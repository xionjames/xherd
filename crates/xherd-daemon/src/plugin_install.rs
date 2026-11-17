//! Installing plugins from a git URL (R4, R5): what a fetched plugin folder
//! holds, and a digest of it so install can check it installs exactly what
//! the user previewed.

use std::path::Path;

use sha2::{Digest, Sha256};
use xherd_core::xap::{Refusal, refuse};

use crate::plugin_dirs::{MAX_BYTES, MAX_DEPTH};

/// What a plugin folder holds.
#[derive(Debug, PartialEq, Eq)]
pub struct Scan {
    /// sha256 over the sorted `path NUL x|- NUL sha256(content)` lines: the
    /// paths, execute bits and contents, not timestamps or owners.
    pub digest: String,
    pub files: usize,
    pub bytes: u64,
}

/// Walk `dir` and digest it. Refused (`bad_plugin_folder`): a symlink or
/// another special file (one could point at the daemon's files), more than 64
/// MiB in total, nesting deeper than 16 folders, an unreadable entry. `.git`
/// is skipped: a fetch leaves history there and it is never installed.
pub fn scan(dir: &Path) -> Result<Scan, Refusal> {
    let mut lines = Vec::new();
    let mut bytes = 0;
    walk(dir, dir, 0, &mut bytes, &mut lines)?;
    lines.sort();
    let mut h = Sha256::new();
    for l in &lines {
        h.update(l.as_bytes());
        h.update(b"\n");
    }
    Ok(Scan {
        digest: hex::encode(h.finalize()),
        files: lines.len(),
        bytes,
    })
}

fn bad(message: impl Into<String>) -> Refusal {
    refuse("bad_plugin_folder", message)
}

fn walk(
    root: &Path,
    dir: &Path,
    depth: usize,
    bytes: &mut u64,
    lines: &mut Vec<String>,
) -> Result<(), Refusal> {
    use std::os::unix::fs::PermissionsExt;
    if depth > MAX_DEPTH {
        return Err(bad("the plugin's folders nest too deeply"));
    }
    let rd = std::fs::read_dir(dir).map_err(|e| bad(format!("cannot read the plugin: {e}")))?;
    for e in rd {
        let e = e.map_err(|e| bad(format!("cannot read the plugin: {e}")))?;
        let path = e.path();
        let rel = path.strip_prefix(root).unwrap_or(&path);
        let t = e
            .file_type()
            .map_err(|e| bad(format!("cannot read {}: {e}", rel.display())))?;
        if depth == 0 && e.file_name() == ".git" {
            continue;
        }
        if t.is_dir() {
            walk(root, &path, depth + 1, bytes, lines)?;
        } else if t.is_file() {
            let meta = path
                .metadata()
                .map_err(|e| bad(format!("cannot read {}: {e}", rel.display())))?;
            *bytes += meta.len();
            if *bytes > MAX_BYTES {
                return Err(bad("the plugin is over 64 MiB"));
            }
            let content = std::fs::read(&path)
                .map_err(|e| bad(format!("cannot read {}: {e}", rel.display())))?;
            let exec = if meta.permissions().mode() & 0o111 != 0 {
                'x'
            } else {
                '-'
            };
            lines.push(format!(
                "{}\0{exec}\0{}",
                rel.display(),
                hex::encode(Sha256::digest(&content))
            ));
        } else {
            return Err(bad(format!(
                "{} is a symlink or special file",
                rel.display()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn plugin() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("lib")).unwrap();
        std::fs::write(d.path().join("xherd-plugin.json"), "{}").unwrap();
        std::fs::write(d.path().join("lib/a.sh"), "echo a").unwrap();
        d
    }

    #[test]
    fn the_digest_is_stable_and_counts_files() {
        let (a, b) = (plugin(), plugin());
        let s = scan(a.path()).unwrap();
        assert_eq!(s.files, 2);
        assert_eq!(s.bytes, 8);
        assert_eq!(s, scan(b.path()).unwrap());
        // .git is not part of what gets installed.
        std::fs::create_dir_all(a.path().join(".git")).unwrap();
        std::fs::write(a.path().join(".git/HEAD"), "x").unwrap();
        assert_eq!(s.digest, scan(a.path()).unwrap().digest);
    }

    #[test]
    fn content_path_and_the_execute_bit_change_the_digest() {
        let d = plugin();
        let base = scan(d.path()).unwrap().digest;
        let f = d.path().join("lib/a.sh");
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        let exec = scan(d.path()).unwrap().digest;
        assert_ne!(base, exec);
        std::fs::write(&f, "echo b").unwrap();
        let edited = scan(d.path()).unwrap().digest;
        assert_ne!(exec, edited);
        std::fs::rename(&f, d.path().join("lib/b.sh")).unwrap();
        assert_ne!(edited, scan(d.path()).unwrap().digest);
    }

    #[test]
    fn a_symlink_is_refused() {
        let d = plugin();
        std::os::unix::fs::symlink("/etc/passwd", d.path().join("lib/link")).unwrap();
        let r = scan(d.path()).unwrap_err();
        assert_eq!(r.code, "bad_plugin_folder");
        assert!(r.message.contains("lib/link"), "{}", r.message);
    }

    #[test]
    fn nesting_too_deep_is_refused() {
        let d = plugin();
        let mut deep = d.path().to_path_buf();
        for _ in 0..=MAX_DEPTH + 1 {
            deep = deep.join("d");
        }
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(scan(d.path()).unwrap_err().code, "bad_plugin_folder");
    }

    #[test]
    fn over_the_size_limit_is_refused() {
        let d = plugin();
        let f = std::fs::File::create(d.path().join("big")).unwrap();
        f.set_len(MAX_BYTES + 1).unwrap();
        let r = scan(d.path()).unwrap_err();
        assert!(r.message.contains("64 MiB"), "{}", r.message);
    }
}
