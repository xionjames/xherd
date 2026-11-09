//! `xherd install` (design §6.7, spec §8.3 level 3): write a pack's commands
//! where a runtime finds them, recording the hash of each written file. A file
//! the user edited since is kept; the new version goes next to it as
//! `<file>.xherd-new`.

use std::path::Path;

use serde::Serialize;
use xherd_core::hash::content_sha256;

use crate::{ActionError, Daemon};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallAction {
    Written,
    Unchanged,
    /// Edited by the user: kept, new version in `<path>.xherd-new`.
    Kept,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub runtime: String,
    pub path: String,
    pub action: InstallAction,
}

fn refuse(code: &'static str, message: impl Into<String>) -> ActionError {
    ActionError {
        code,
        message: message.into(),
    }
}

/// The commands go where the agents find them: written as the agent user
/// (R-1203).
async fn write(path: &Path, text: &str) -> Result<(), ActionError> {
    xherd_core::runner::write_file(path, text.as_bytes(), None)
        .await
        .map_err(|e| refuse("io", format!("{}: {e}", path.display())))
}

impl Daemon {
    /// Install `pack` (default xherd) for `runtime` (default: every runtime
    /// the pack supports). `env` resolves `${VAR}` and `HOME` in destinations.
    pub async fn install(
        &self,
        runtime: Option<&str>,
        pack: Option<&str>,
        env: &(dyn Fn(&str) -> Option<String> + Sync),
    ) -> Result<Vec<Installed>, ActionError> {
        let name = pack.unwrap_or(xherd_flow::BUILTIN);
        let (Some(p), true) = (self.packs.get(name), name == xherd_flow::BUILTIN) else {
            return Err(refuse("unknown_pack", format!("no pack {name}")));
        };
        let runtimes: Vec<&str> = match runtime {
            Some(r) if p.install.contains_key(r) => vec![r],
            Some(r) => {
                return Err(refuse(
                    "unknown_runtime",
                    format!("pack {name} has nothing to install for runtime {r}"),
                ));
            }
            None => p.install.keys().map(String::as_str).collect(),
        };
        let mut out = Vec::new();
        for rt in runtimes {
            let plan = p
                .plan_install(rt, xherd_flow::install::BUILTIN_FILES, env)
                .map_err(|e| refuse("install", e))?;
            for f in plan {
                let key = f.path.to_string_lossy().into_owned();
                let new = content_sha256(f.text.as_bytes());
                let current = xherd_core::runner::read_file(&f.path)
                    .await
                    .ok()
                    .flatten()
                    .map(|b| content_sha256(&b));
                let recorded = self
                    .store
                    .install_sha(rt, name, &key)
                    .await
                    .map_err(|e| refuse("store", e.to_string()))?;
                let action = match current {
                    Some(c) if c == new => InstallAction::Unchanged,
                    Some(c) if recorded.as_deref() != Some(c.as_str()) => {
                        let mut side = f.path.clone().into_os_string();
                        side.push(".xherd-new");
                        write(Path::new(&side), &f.text).await?;
                        InstallAction::Kept
                    }
                    _ => {
                        write(&f.path, &f.text).await?;
                        InstallAction::Written
                    }
                };
                if action != InstallAction::Kept && recorded.as_deref() != Some(new.as_str()) {
                    self.store
                        .put_install(rt, name, &key, &new)
                        .await
                        .map_err(|e| refuse("store", e.to_string()))?;
                }
                out.push(Installed {
                    runtime: rt.into(),
                    path: key,
                    action,
                });
            }
        }
        Ok(out)
    }
}
