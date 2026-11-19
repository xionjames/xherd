//! `xherd install` (design §6.7, spec §8.3 level 3): write a pack's commands
//! where a runtime finds them, recording the hash of each written file. A file
//! the user edited since is kept; the new version goes next to it as
//! `<file>.xherd-new`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use xherd_core::hash::content_sha256;

use crate::{ActionError, Daemon};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallAction {
    Written,
    Unchanged,
    /// Edited by the user: kept, new version in `<path>.xherd-new`.
    Kept,
    /// The user chose to keep their file over this pack text: left alone.
    Declined,
    /// Recorded by an earlier install, no longer in the pack: left alone.
    Obsolete,
    /// Obsolete and deleted (with its emptied folders).
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub runtime: String,
    pub path: String,
    pub action: InstallAction,
}

/// What a pack file is like, from its hashes (see [`classify`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileState {
    /// Not on disk: install writes it.
    Missing,
    /// On disk with the pack's current text.
    Current,
    /// An older pack text the user never touched: install upgrades it.
    Outdated,
    /// The user chose to keep their own file over this pack text.
    Declined,
    /// Changed by the user since install: kept, the pack text goes next to it.
    Edited,
    /// Recorded by an earlier install, no longer in the pack.
    Obsolete,
}

/// One file of a pack for one runtime. `text` and `new` (its sha256) are the
/// pack's version, empty for an obsolete file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackFile {
    pub runtime: String,
    pub path: String,
    pub state: FileState,
    /// Obsolete only: the file is on disk and differs from what was recorded.
    pub edited: bool,
    #[serde(skip)]
    pub text: String,
    #[serde(skip)]
    pub new: String,
}

/// The state of a file the pack still ships, from the sha256 of the pack text
/// (`new`), of the file on disk (`cur`), of what the last install recorded
/// and of the version the user declined.
pub fn classify(
    new: &str,
    cur: Option<&str>,
    recorded: Option<&str>,
    declined: Option<&str>,
) -> FileState {
    match cur {
        None => FileState::Missing,
        Some(c) if c == new => FileState::Current,
        Some(c) if recorded == Some(c) => FileState::Outdated,
        Some(_) if declined == Some(new) => FileState::Declined,
        Some(_) => FileState::Edited,
    }
}

/// What the user decided for one file of the review (design §6.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Choice {
    /// Edited file: replace it with the pack's text.
    UseNew,
    /// Edited file: keep it and stop offering this pack text.
    KeepMine,
    /// Obsolete file: delete it.
    Remove,
    /// Obsolete file: leave it and forget it.
    Keep,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackChoice {
    pub runtime: String,
    pub path: String,
    pub choice: Choice,
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
    /// Every file of the built-in pack for `runtime`, classified, then the
    /// obsolete ones (recorded, no longer in the pack), by path.
    pub async fn pack_files(
        &self,
        runtime: &str,
        env: &(dyn Fn(&str) -> Option<String> + Sync),
    ) -> Result<Vec<PackFile>, ActionError> {
        let name = xherd_flow::BUILTIN;
        let p = self
            .packs
            .get(name)
            .ok_or_else(|| refuse("unknown_pack", format!("no pack {name}")))?;
        let plan = p
            .plan_install(runtime, xherd_flow::install::BUILTIN_FILES, env)
            .map_err(|e| refuse("install", e))?;
        let records = self
            .store
            .installs(runtime, name)
            .await
            .map_err(|e| refuse("store", e.to_string()))?;
        let disk = |path: std::path::PathBuf| async move {
            xherd_core::runner::read_file(&path)
                .await
                .ok()
                .flatten()
                .map(|b| content_sha256(&b))
        };
        let mut out = Vec::new();
        for f in plan {
            let path = f.path.to_string_lossy().into_owned();
            let new = content_sha256(f.text.as_bytes());
            let cur = disk(f.path).await;
            let rec = records.iter().find(|r| r.path == path);
            let state = classify(
                &new,
                cur.as_deref(),
                rec.map(|r| r.sha256.as_str()),
                rec.and_then(|r| r.declined.as_deref()),
            );
            out.push(PackFile {
                runtime: runtime.into(),
                path,
                state,
                edited: false,
                text: f.text,
                new,
            });
        }
        for r in records {
            if out.iter().any(|f| f.path == r.path) {
                continue;
            }
            let cur = disk(r.path.clone().into()).await;
            out.push(PackFile {
                runtime: runtime.into(),
                edited: cur.is_some_and(|c| c != r.sha256),
                path: r.path,
                state: FileState::Obsolete,
                text: String::new(),
                new: String::new(),
            });
        }
        Ok(out)
    }

    /// The files that need the user: edited ones (kept, new text beside them)
    /// and obsolete ones, for every runtime of the pack (R5, R6).
    pub async fn pack_review(
        &self,
        env: &(dyn Fn(&str) -> Option<String> + Sync),
    ) -> Result<Vec<PackFile>, ActionError> {
        let mut out = Vec::new();
        for rt in self.pack_runtimes() {
            out.extend(
                self.pack_files(&rt, env)
                    .await?
                    .into_iter()
                    .filter(|f| matches!(f.state, FileState::Edited | FileState::Obsolete)),
            );
        }
        Ok(out)
    }

    /// Apply the user's choices. Every choice is checked against
    /// [`Daemon::pack_review`] first: a file not listed there, or a choice
    /// that does not fit its state, refuses the whole request (`not_pending`)
    /// and changes nothing. Paths come from the review, never from the request.
    pub async fn resolve_pack_files(
        &self,
        choices: &[PackChoice],
        env: &(dyn Fn(&str) -> Option<String> + Sync),
    ) -> Result<(), ActionError> {
        let name = xherd_flow::BUILTIN;
        let review = self.pack_review(env).await?;
        let mut picked = Vec::new();
        for c in choices {
            let f = review
                .iter()
                .find(|f| f.runtime == c.runtime && f.path == c.path)
                .filter(|f| match f.state {
                    FileState::Edited => matches!(c.choice, Choice::UseNew | Choice::KeepMine),
                    _ => matches!(c.choice, Choice::Remove | Choice::Keep),
                })
                .ok_or_else(|| {
                    refuse(
                        "not_pending",
                        format!("{}: not waiting for a choice", c.path),
                    )
                })?;
            picked.push((f, c.choice));
        }
        let store = |e: xherd_store::StoreError| refuse("store", e.to_string());
        for (f, choice) in picked {
            let rt = f.runtime.as_str();
            let root = self
                .packs
                .get(name)
                .ok_or_else(|| refuse("unknown_pack", format!("no pack {name}")))?
                .install_root(rt, env)
                .map_err(|e| refuse("install", e))?;
            let path = Path::new(&f.path);
            let side = format!("{}.xherd-new", f.path);
            let gone = |e: std::io::Error, p: &str| refuse("io", format!("{p}: {e}"));
            match choice {
                Choice::UseNew => {
                    write(path, &f.text).await?;
                    self.store
                        .put_install(rt, name, &f.path, &f.new)
                        .await
                        .map_err(store)?;
                    xherd_core::runner::remove_file(Path::new(&side), &root)
                        .await
                        .map_err(|e| gone(e, &side))?;
                }
                Choice::KeepMine => {
                    xherd_core::runner::remove_file(Path::new(&side), &root)
                        .await
                        .map_err(|e| gone(e, &side))?;
                    self.store
                        .decline_install(rt, name, &f.path, &f.new)
                        .await
                        .map_err(store)?;
                }
                Choice::Remove => {
                    xherd_core::runner::remove_file(path, &root)
                        .await
                        .map_err(|e| gone(e, &f.path))?;
                    xherd_core::runner::remove_file(Path::new(&side), &root)
                        .await
                        .map_err(|e| gone(e, &side))?;
                    self.store
                        .delete_install(rt, name, &f.path)
                        .await
                        .map_err(store)?;
                }
                Choice::Keep => {
                    self.store
                        .delete_install(rt, name, &f.path)
                        .await
                        .map_err(store)?;
                }
            }
        }
        Ok(())
    }

    /// Install `pack` (default xherd) for `runtime` (default: every runtime
    /// the pack supports). `env` resolves `${VAR}` and `HOME` in destinations.
    /// Files the pack no longer ships are listed as obsolete and deleted only
    /// with `remove_obsolete`.
    pub async fn install(
        &self,
        runtime: Option<&str>,
        pack: Option<&str>,
        env: &(dyn Fn(&str) -> Option<String> + Sync),
        remove_obsolete: bool,
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
            // Every runtime switched on that the pack supports.
            None => p
                .install
                .keys()
                .map(String::as_str)
                .filter(|r| self.plugin_enabled(r) && self.plugins.get(r).is_some())
                .collect(),
        };
        let mut out = Vec::new();
        for rt in runtimes {
            let root = p.install_root(rt, env).map_err(|e| refuse("install", e))?;
            for f in self.pack_files(rt, env).await? {
                let action = match f.state {
                    FileState::Current => InstallAction::Unchanged,
                    FileState::Declined => InstallAction::Declined,
                    FileState::Edited => {
                        write(Path::new(&format!("{}.xherd-new", f.path)), &f.text).await?;
                        InstallAction::Kept
                    }
                    FileState::Missing | FileState::Outdated => {
                        write(Path::new(&f.path), &f.text).await?;
                        InstallAction::Written
                    }
                    FileState::Obsolete if remove_obsolete => {
                        xherd_core::runner::remove_file(Path::new(&f.path), &root)
                            .await
                            .map_err(|e| refuse("io", format!("{}: {e}", f.path)))?;
                        self.store
                            .delete_install(rt, name, &f.path)
                            .await
                            .map_err(|e| refuse("store", e.to_string()))?;
                        InstallAction::Removed
                    }
                    FileState::Obsolete => InstallAction::Obsolete,
                };
                if matches!(action, InstallAction::Written | InstallAction::Unchanged) {
                    let recorded = self
                        .store
                        .install_sha(rt, name, &f.path)
                        .await
                        .map_err(|e| refuse("store", e.to_string()))?;
                    if recorded.as_deref() != Some(f.new.as_str()) {
                        self.store
                            .put_install(rt, name, &f.path, &f.new)
                            .await
                            .map_err(|e| refuse("store", e.to_string()))?;
                    }
                }
                out.push(Installed {
                    runtime: rt.into(),
                    path: f.path,
                    action,
                });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_every_row_of_the_table() {
        use FileState::*;
        // (new, cur, recorded, declined)
        assert_eq!(classify("n", None, Some("n"), None), Missing);
        assert_eq!(classify("n", Some("n"), None, None), Current);
        assert_eq!(classify("n", Some("n"), Some("o"), Some("x")), Current);
        assert_eq!(classify("n", Some("o"), Some("o"), None), Outdated);
        assert_eq!(classify("n", Some("o"), Some("o"), Some("n")), Outdated);
        assert_eq!(classify("n", Some("m"), Some("o"), Some("n")), Declined);
        assert_eq!(classify("n", Some("m"), Some("o"), None), Edited);
        // A different pack text than the declined one asks again.
        assert_eq!(classify("n2", Some("m"), Some("o"), Some("n")), Edited);
        // Never recorded, but present and different: the user's own file.
        assert_eq!(classify("n", Some("m"), None, None), Edited);
    }
}
