//! `xherd backup create|verify|restore` (R-1401, R-1403, R-1404). They work on
//! the data root directly; `restore` needs the daemon stopped.

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use xherd_core::config::Config;
use xherd_core::paths::Paths;
use xherd_daemon::backup::{self, RestoreOptions};
use xherd_git::Git;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

fn sandbox() -> bool {
    std::env::var("XHERD_SANDBOX").is_ok_and(|v| v == "1")
}

fn daemon_running(cfg: &Config) -> bool {
    format!("127.0.0.1:{}", cfg.port)
        .parse::<SocketAddr>()
        .ok()
        .is_some_and(|a| TcpStream::connect_timeout(&a, Duration::from_millis(300)).is_ok())
}

fn show(warnings: &[String]) {
    for w in warnings {
        eprintln!("  note: {w}");
    }
}

pub fn create(to: Option<PathBuf>) -> i32 {
    let paths = Paths::from_env();
    let cfg = match Config::load(&paths.config()) {
        Ok((c, _)) => c,
        Err(e) => {
            eprintln!("xherd backup create: {e}");
            return 2;
        }
    };
    if !paths.db().exists() {
        eprintln!(
            "xherd backup create: no database at {} (has the daemon run here? XHERD_HOME)",
            paths.db().display()
        );
        return 2;
    }
    let dest = to.unwrap_or_else(|| backup::default_dir(&paths, &cfg, sandbox()));
    rt().block_on(async {
        let store = match xherd_store::Store::open(&paths.db(), &paths.backups()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("xherd backup create: {e}");
                return 2;
            }
        };
        match backup::create(
            &paths,
            &cfg.workspace_root(),
            &store,
            &Git::new("git"),
            &dest,
            "manual",
            xherd_core::now_ms(),
        )
        .await
        {
            Ok(c) => {
                println!("{} ({} agent(s))", c.path.display(), c.agents);
                show(&c.warnings);
                0
            }
            Err(e) => {
                eprintln!("xherd backup create: {e}");
                1
            }
        }
    })
}

pub fn verify(archive: &Path) -> i32 {
    let paths = Paths::from_env();
    if let Err(e) = paths.ensure() {
        eprintln!("xherd backup verify: {e}");
        return 2;
    }
    match rt().block_on(backup::verify(
        archive,
        &paths.scratch(),
        &Git::new("git"),
        xherd_core::now_ms(),
    )) {
        Ok(m) => {
            println!(
                "ok: {} file(s), {} agent(s), made by xherd {}, workspace {}",
                m.files.len(),
                m.agents.len(),
                m.xherd_version,
                m.workspace
            );
            0
        }
        Err(e) => {
            eprintln!("xherd backup verify: {e}");
            1
        }
    }
}

pub fn restore(archive: &Path, workspace: Option<String>, maps: Vec<String>, force: bool) -> i32 {
    let paths = Paths::from_env();
    // A data root that has no config yet has no daemon of its own.
    if paths.config().exists()
        && let Ok((cfg, _)) = Config::load(&paths.config())
        && daemon_running(&cfg)
    {
        eprintln!("xherd backup restore: the daemon is running; stop it first");
        return 1;
    }
    let mut parsed = Vec::new();
    for m in maps {
        match m.split_once('=') {
            Some((a, b)) if a.starts_with('/') && b.starts_with('/') => {
                parsed.push((a.to_string(), b.to_string()))
            }
            _ => {
                eprintln!(
                    "xherd backup restore: --map wants OLD=NEW, both absolute paths (got {m:?})"
                );
                return 2;
            }
        }
    }
    let opts = RestoreOptions {
        maps: parsed,
        workspace,
        force,
    };
    match rt().block_on(backup::restore(
        &paths,
        &Git::new("git"),
        archive,
        &opts,
        xherd_core::now_ms(),
    )) {
        Ok(r) => {
            println!(
                "restored {} agent(s), paused: {}",
                r.agents.len(),
                r.agents.join(", ")
            );
            show(&r.warnings);
            println!("start the daemon, then resume the agents you want running");
            0
        }
        Err(e) => {
            eprintln!("xherd backup restore: {e}");
            1
        }
    }
}
