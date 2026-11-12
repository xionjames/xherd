//! Where external plugins run from (R-545, revised). In the two-user
//! container the daemon owns the database, `secrets.json` and the user token,
//! so plugin code runs as the agents' user instead: it can use the agents'
//! CLI logins (`gh`, `glab`, …) and cannot read the daemon's files. That user
//! cannot enter the daemon's private data folder, so the plugins are copied
//! to a folder it can read and run but not change, and their config and state
//! folders live in the agents' home. Natively (one user) nothing moves.

use std::io;
use std::path::{Path, PathBuf};

use xherd_core::paths::Paths;
use xherd_plugin::Broken;

/// The shared, agent-readable folder the daemon owns (`XHERD_SHARED`).
fn shared() -> PathBuf {
    std::env::var_os("XHERD_SHARED")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(xherd_core::container::SHARED))
}

/// Commands go to the agents' user through the runner (two-user container).
fn two_users() -> bool {
    xherd_core::runner::runner_socket().is_some()
}

/// The folder external plugins are loaded from: `<data>/plugins` itself, or
/// in the two-user container a copy of it the agents' user can run. Also the
/// plugins that could not be copied there (listed as broken, R-507).
pub fn external_dir(paths: &Paths) -> (PathBuf, Vec<Broken>) {
    if !two_users() {
        return (paths.plugins(), Vec::new());
    }
    let dst = shared().join("plugins");
    match mirror(&paths.plugins(), &dst) {
        Ok(skipped) => (dst, skipped),
        Err(e) => {
            tracing::warn!(error = %e, "could not copy the plugins where the agents' user can run them");
            (paths.plugins(), Vec::new())
        }
    }
}

/// `XHERD_PLUGIN_CONFIG_DIR` and `XHERD_PLUGIN_STATE_DIR` of a plugin: the
/// plugin writes there, so in the two-user container they are in the agents'
/// home (the plugin creates them itself).
pub fn config_dir(paths: &Paths, plugin: &str) -> PathBuf {
    match agent_home() {
        Some(h) => h.join("config").join(plugin),
        None => paths.plugin_config(plugin),
    }
}

pub fn state_dir(paths: &Paths, plugin: &str) -> PathBuf {
    match agent_home() {
        Some(h) => h.join("state").join(plugin),
        None => paths.plugin_state(plugin),
    }
}

fn agent_home() -> Option<PathBuf> {
    if !two_users() {
        return None;
    }
    xherd_core::runner::agent_env("HOME")
        .map(|h| PathBuf::from(h).join(".local/share/xherd-plugins"))
}

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DEPTH: usize = 16;

/// Replace `dst` with a copy of `src`: folders 0755, files 0555 when `src`'s
/// has an execute bit, else 0444, so the agents' user can read and run them
/// but not change them. Symlinks are skipped (one could point at the daemon's
/// files). Each plugin folder is copied on its own: one that cannot be (over
/// 64 MiB, nested too deeply, unreadable) is left out and returned, and the
/// others are copied anyway. A missing `src` leaves an empty `dst`.
pub fn mirror(src: &Path, dst: &Path) -> io::Result<Vec<Broken>> {
    use std::os::unix::fs::PermissionsExt;
    if dst.exists() {
        // Files are read-only for everyone, so make the folders writable first.
        let _ = chmod_tree(dst);
        std::fs::remove_dir_all(dst)?;
    }
    std::fs::create_dir_all(dst)?;
    std::fs::set_permissions(dst, std::fs::Permissions::from_mode(0o755))?;
    let mut skipped = Vec::new();
    let Ok(rd) = std::fs::read_dir(src) else {
        return Ok(skipped);
    };
    for e in rd.flatten() {
        let (from, to) = (e.path(), dst.join(e.file_name()));
        if !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let copied = std::fs::create_dir(&to).and_then(|()| {
            let mut total = 0u64;
            copy(&from, &to, 0, &mut total)?;
            std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o755))
        });
        if let Err(error) = copied {
            let name = e.file_name().to_string_lossy().into_owned();
            tracing::warn!(plugin = %name, %error, "plugin not copied where the agents' user can run it");
            let _ = std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o755));
            let _ = chmod_tree(&to);
            let _ = std::fs::remove_dir_all(&to);
            skipped.push(Broken {
                name,
                dir: from,
                error: format!("not copied for the agents' user: {error}"),
            });
        }
    }
    Ok(skipped)
}

fn chmod_tree(p: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for e in std::fs::read_dir(p)?.flatten() {
        let path = e.path();
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
            chmod_tree(&path)?;
        }
    }
    Ok(())
}

fn copy(src: &Path, dst: &Path, depth: usize, total: &mut u64) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let Ok(rd) = std::fs::read_dir(src) else {
        return Ok(());
    };
    if depth > MAX_DEPTH {
        return Err(io::Error::other("plugin folders nest too deeply"));
    }
    for e in rd.flatten() {
        let (from, to) = (e.path(), dst.join(e.file_name()));
        let Ok(t) = e.file_type() else { continue };
        if t.is_symlink() {
            tracing::warn!(path = %from.display(), "plugin symlink not copied");
        } else if t.is_dir() {
            std::fs::create_dir(&to)?;
            copy(&from, &to, depth + 1, total)?;
            std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o755))?;
        } else if t.is_file() {
            let mode = from.metadata()?.permissions().mode();
            *total += from.metadata()?.len();
            if *total > MAX_BYTES {
                return Err(io::Error::other("a plugin is over 64 MiB"));
            }
            std::fs::copy(&from, &to)?;
            let perm = if mode & 0o111 != 0 { 0o555 } else { 0o444 };
            std::fs::set_permissions(&to, std::fs::Permissions::from_mode(perm))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(p: &Path) -> u32 {
        p.metadata().unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_mirror_is_readable_and_runnable_but_not_writable_and_replaces_the_old_one() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("src"), d.path().join("dst"));
        std::fs::create_dir_all(src.join("p/lib")).unwrap();
        std::fs::write(src.join("p/xherd-plugin.json"), "{}").unwrap();
        std::fs::write(src.join("p/run.sh"), "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(src.join("p/run.sh"), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        std::fs::write(src.join("p/lib/a.txt"), "a").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", src.join("p/link")).unwrap();

        assert!(mirror(&src, &dst).unwrap().is_empty());
        assert_eq!(mode(&dst), 0o755);
        assert_eq!(mode(&dst.join("p/lib")), 0o755);
        assert_eq!(mode(&dst.join("p/xherd-plugin.json")), 0o444);
        assert_eq!(mode(&dst.join("p/run.sh")), 0o555);
        assert!(!dst.join("p/link").exists(), "symlinks are not copied");
        assert_eq!(
            std::fs::read_to_string(dst.join("p/lib/a.txt")).unwrap(),
            "a"
        );

        // A second run replaces the copy, read-only files included.
        std::fs::remove_dir_all(src.join("p/lib")).unwrap();
        mirror(&src, &dst).unwrap();
        assert!(!dst.join("p/lib").exists());
        // No source folder: an empty copy.
        mirror(&d.path().join("none"), &dst).unwrap();
        assert!(std::fs::read_dir(&dst).unwrap().next().is_none());
    }

    /// One plugin that cannot be copied costs only itself.
    #[test]
    fn a_plugin_that_cannot_be_copied_leaves_the_others() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("src"), d.path().join("dst"));
        std::fs::create_dir_all(src.join("good")).unwrap();
        std::fs::write(src.join("good/xherd-plugin.json"), "{}").unwrap();
        let mut deep = src.join("deep");
        for _ in 0..=MAX_DEPTH + 1 {
            deep = deep.join("d");
        }
        std::fs::create_dir_all(&deep).unwrap();

        let skipped = mirror(&src, &dst).unwrap();
        assert_eq!(
            skipped.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
            ["deep"]
        );
        assert!(skipped[0].error.contains("nest too deeply"), "{skipped:?}");
        assert!(dst.join("good/xherd-plugin.json").exists());
        assert!(!dst.join("deep").exists(), "a half copy is removed");
    }

    #[test]
    fn natively_nothing_moves() {
        let paths = Paths::at(PathBuf::from("/data"));
        assert_eq!(external_dir(&paths).0, PathBuf::from("/data/plugins"));
        assert_eq!(
            config_dir(&paths, "x"),
            PathBuf::from("/data/plugin-config/x")
        );
        assert_eq!(
            state_dir(&paths, "x"),
            PathBuf::from("/data/plugin-state/x")
        );
    }
}
