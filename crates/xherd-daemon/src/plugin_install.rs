//! Installing plugins from a git URL (R4, R5): what a fetched plugin folder
//! holds, and a digest of it so install can check it installs exactly what
//! the user previewed.

use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use xherd_core::xap::{Refusal, refuse};
use xherd_git::{Git, GitError};
use xherd_plugin::manifest::parse_version;
use xherd_plugin::{Kind, MANIFEST_FILE, Manifest, PluginSource};

use crate::create::refuse as refuse_action;
use crate::plugin_dirs::{MAX_BYTES, MAX_DEPTH};
use crate::repos::{CLONE_TIMEOUT, remove_tree};
use crate::{ActionError, Daemon};

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

/// What plugins run with (R3, R-545), shown with every preview.
pub const WARNING: &str = "A plugin runs commands with the rights of the agents' user, \
    with no further sandbox. Install only code you trust.";

/// `POST /plugins/preview` (R2, R3, R4): what installing a source would put
/// on disk, found without running any of its code.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub url: String,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    /// The sub-folder holding the manifest; `None` is the repository root
    /// (and, with `choices`, not chosen yet).
    pub path: Option<String>,
    /// The commit the ref resolved to.
    pub sha: String,
    #[serde(flatten)]
    pub plugin: Option<PluginView>,
    /// Several plugins in the repository: send one `path` back.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginView {
    pub name: String,
    pub kind: Kind,
    pub version: String,
    pub label: String,
    pub description: String,
    pub min_xherd_version: Option<String>,
    /// This xherd satisfies `min_xherd_version`.
    pub compatible: bool,
    /// What install checks again (R4): see [`scan`].
    pub digest: String,
    pub files: usize,
    pub bytes: u64,
    /// The plugin takes the place of a built-in or an installed one.
    pub replaces: Option<&'static str>,
    pub settings: Vec<SettingView>,
    pub runs: Vec<Run>,
    pub warning: &'static str,
}

#[derive(Debug, Serialize)]
pub struct SettingView {
    pub key: String,
    pub label: String,
    pub secret: bool,
    pub required: bool,
}

/// One argv the plugin declares.
#[derive(Debug, Serialize)]
pub struct Run {
    pub what: String,
    pub argv: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Choice {
    pub path: String,
    pub name: String,
    pub kind: Option<Kind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What `plugin_install` put in `<data>/plugins`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub name: String,
    pub kind: Kind,
    pub version: String,
    pub url: String,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    pub path: Option<String>,
    pub sha: String,
    pub digest: String,
}

/// Remove what an interrupted install or update left in `plugins`
/// (`.<name>.staging-*`, `.<name>.old-*`): the registry would list them as
/// broken plugins.
pub(crate) fn clean_staging(plugins: &Path) {
    let Ok(rd) = std::fs::read_dir(plugins) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && (name.contains(".staging-") || name.contains(".old-")) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Copy a scanned plugin folder (no symlinks, see [`scan`]) without `.git`:
/// folders 0755, files 0755 with an execute bit and 0644 without.
fn copy_plugin(src: &Path, dst: &Path, depth: usize) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let (from, to) = (e.path(), dst.join(e.file_name()));
        let t = e.file_type()?;
        if depth == 0 && e.file_name() == ".git" {
            continue;
        }
        if t.is_dir() {
            std::fs::create_dir(&to)?;
            copy_plugin(&from, &to, depth + 1)?;
        } else if t.is_file() {
            std::fs::copy(&from, &to)?;
            let exec = from.metadata()?.permissions().mode() & 0o111 != 0;
            std::fs::set_permissions(
                &to,
                std::fs::Permissions::from_mode(if exec { 0o755 } else { 0o644 }),
            )?;
        } else {
            return Err(std::io::Error::other("not a plain file or folder"));
        }
    }
    Ok(())
}

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}

/// Copy the checked folder `dir` to `<plugins>/.<name>.staging-<nanos>` and
/// return it; nothing is left behind when the copy fails.
fn stage_plugin(plugins: &Path, name: &str, dir: &Path) -> Result<PathBuf, ActionError> {
    let io = |e: std::io::Error| refuse_action("io", format!("{name}: {e}"));
    std::fs::create_dir_all(plugins).map_err(io)?;
    let staging = plugins.join(format!(".{name}.staging-{}", nanos()));
    let copied = std::fs::create_dir(&staging).and_then(|()| copy_plugin(dir, &staging, 0));
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(io(e));
    }
    Ok(staging)
}

/// What `plugin_update` and `plugin_update_preview` say about the installed
/// plugin: its recorded source.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recorded {
    pub url: String,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    pub path: Option<String>,
    pub sha: Option<String>,
    pub version: Option<String>,
    pub digest: Option<String>,
}

impl Recorded {
    fn of(r: &xherd_store::PluginRecord) -> Recorded {
        let text = |k: &str| r.data[k].as_str().map(str::to_string);
        Recorded {
            url: r.source.clone(),
            git_ref: text("ref"),
            path: text("path"),
            sha: r.sha.clone(),
            version: text("version"),
            digest: text("digest"),
        }
    }
}

/// `POST /plugins/{name}/update` without a digest (R8): the new source as in
/// a preview, what is installed now, and the manifest's diff.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePreview {
    #[serde(flatten)]
    pub preview: Preview,
    pub installed: Recorded,
    /// Unified diff of the installed manifest against the new one; empty when
    /// they are equal.
    pub manifest_diff: String,
}

/// What `plugin_update` changed.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Updated {
    pub name: String,
    pub version: String,
    pub url: String,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    pub path: Option<String>,
    pub sha: String,
    pub digest: String,
    pub previous_sha: Option<String>,
}

/// What `plugin_remove` did.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    pub name: String,
    /// Its settings and secrets were dropped too.
    pub forgot: bool,
}

/// The manifest as pretty JSON (the same text for the same content), so a
/// diff shows changes, not formatting.
fn pretty_manifest(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .map(|s| s + "\n")
        .unwrap_or_else(|_| text.to_string())
}

/// Where the manifest is in a fetched repository.
enum Found {
    One(PathBuf, Option<String>),
    Many(Vec<Choice>),
}

impl Daemon {
    /// The checked source, or `plugin_install_off`. Tests may use `file://`.
    pub(crate) fn plugin_source(
        &self,
        url: &str,
        git_ref: Option<&str>,
        path: Option<&str>,
    ) -> Result<PluginSource, ActionError> {
        if !self.plugin_install {
            return Err(refuse_action(
                "plugin_install_off",
                "installing plugins is turned off (XHERD_PLUGIN_INSTALL=off)",
            ));
        }
        let url = url.trim();
        if self.allow_local_plugin_sources && url.starts_with("file://") {
            let mut s = PluginSource::parse("https://example.com/o/r", git_ref, path)?;
            s.url = url.to_string();
            return Ok(s);
        }
        Ok(PluginSource::parse(url, git_ref, path)?)
    }

    /// Fetch `src` into a scratch folder (as the agents' user, R12), run `f`
    /// on it and always remove the folder.
    pub(crate) async fn with_fetched<T>(
        &self,
        src: &PluginSource,
        f: impl FnOnce(&Path, String) -> Result<T, ActionError>,
    ) -> Result<T, ActionError> {
        let repos = self.config.workspace_root().join("repos");
        std::fs::create_dir_all(&repos)
            .map_err(|e| refuse_action("io", format!("{}: {e}", repos.display())))?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dest = repos.join(format!(".xherd-plugin-{nanos}"));
        let git = Git::new("git").with_timeout(CLONE_TIMEOUT);
        let fetched = git
            .fetch_shallow(&src.url, src.git_ref.as_deref(), &dest)
            .await;
        let out = match fetched {
            Ok(sha) => f(&dest, sha),
            Err(e) => {
                let e = match e {
                    GitError::Failed { stderr, .. } => stderr,
                    e => e.to_string(),
                };
                Err(refuse_action(
                    "fetch_failed",
                    format!(
                        "cannot fetch {}: {e}. Check Settings → Git access for the host",
                        src.url
                    ),
                ))
            }
        };
        remove_tree(&dest).await;
        out
    }

    /// R2, R3, R4: fetch `url` at `git_ref` (default branch when `None`) and
    /// describe the plugin in it, or the plugins to choose from.
    pub async fn plugin_preview(
        &self,
        url: &str,
        git_ref: Option<&str>,
        path: Option<&str>,
    ) -> Result<Preview, ActionError> {
        let src = self.plugin_source(url, git_ref, path)?;
        self.with_fetched(&src, |root, sha| {
            let (git_ref, url) = (src.git_ref.clone(), src.url.clone());
            match locate(root, src.path.as_deref())? {
                Found::Many(choices) => Ok(Preview {
                    url,
                    git_ref,
                    path: None,
                    sha,
                    plugin: None,
                    choices,
                }),
                Found::One(dir, path) => Ok(Preview {
                    url,
                    git_ref,
                    path,
                    sha,
                    plugin: Some(self.inspect(&dir)?),
                    choices: Vec::new(),
                }),
            }
        })
        .await
    }

    /// R4, R5, R6, R11: install what `plugin_preview` showed. The source is
    /// fetched again and must digest to `expect_digest` (`changed_since_preview`
    /// otherwise, nothing written); an installed plugin of that name is not
    /// replaced (`plugin_exists`). The folder is copied under a staging name and
    /// renamed, so `<data>/plugins/<name>` is whole or absent. Needs a restart
    /// to load (no live reload).
    pub async fn plugin_install(
        &self,
        url: &str,
        git_ref: Option<&str>,
        path: Option<&str>,
        expect_digest: &str,
    ) -> Result<Installed, ActionError> {
        let src = self.plugin_source(url, git_ref, path)?;
        let _write = self.plugins_write.lock().await;
        let plugins = self.paths.plugins();
        let (view, found_path, sha) = self
            .with_fetched(&src, |root, sha| {
                let (dir, found_path) = match locate(root, src.path.as_deref())? {
                    Found::One(dir, p) => (dir, p),
                    Found::Many(_) => {
                        return Err(refuse_action(
                            "no_manifest",
                            "several plugins in the repository: choose one with a path",
                        ));
                    }
                };
                let view = self.inspect(&dir)?;
                if view.digest != expect_digest {
                    return Err(refuse_action(
                        "changed_since_preview",
                        "the source changed since the preview: preview it again",
                    ));
                }
                if view.replaces == Some("external") {
                    return Err(refuse_action(
                        "plugin_exists",
                        format!("a plugin named {} is already installed", view.name),
                    ));
                }
                let staging = stage_plugin(&plugins, &view.name, &dir)?;
                if let Err(e) = std::fs::rename(&staging, plugins.join(&view.name)) {
                    let _ = std::fs::remove_dir_all(&staging);
                    return Err(refuse_action("io", format!("{}: {e}", view.name)));
                }
                Ok((view, found_path, sha))
            })
            .await?;
        let now = self.clock.now();
        let kind = serde_json::to_value(view.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let record = xherd_store::PluginRecord {
            name: view.name.clone(),
            kind,
            source: src.url.clone(),
            sha: Some(sha.clone()),
            enabled: true,
            data: serde_json::json!({
                "ref": src.git_ref,
                "path": found_path,
                "digest": view.digest,
                "version": view.version,
                "installedAt": now,
            }),
        };
        let recorded = async {
            self.store.upsert_plugin_record(&record).await?;
            self.store
                .insert_event(xherd_store::Event {
                    id: xherd_core::new_id(),
                    job_id: String::new(),
                    kind: "plugin-install".into(),
                    payload: Some(serde_json::json!({
                        "name": view.name, "source": src.url, "sha": sha,
                        "digest": view.digest, "version": view.version,
                    })),
                    at: now,
                })
                .await
        }
        .await;
        if let Err(e) = recorded {
            // No record, no plugin: do not leave an install nobody can see.
            let _ = std::fs::remove_dir_all(plugins.join(&view.name));
            return Err(refuse_action(
                "io",
                format!("cannot record the install: {e}"),
            ));
        }
        Ok(Installed {
            name: view.name,
            kind: view.kind,
            version: view.version,
            url: src.url,
            git_ref: src.git_ref,
            path: found_path,
            sha,
            digest: view.digest,
        })
    }

    /// The record of an installed external plugin: `builtin_plugin` for a
    /// built-in, `unknown_plugin` when it was not installed from a source.
    async fn plugin_record(&self, name: &str) -> Result<xherd_store::PluginRecord, ActionError> {
        if self.plugins.get(name).is_some_and(|p| p.builtin()) {
            return Err(refuse_action(
                "builtin_plugin",
                format!("{name} is built in: it cannot be changed or removed"),
            ));
        }
        let records = self
            .store
            .plugin_records()
            .await
            .map_err(|e| refuse_action("io", e.to_string()))?;
        records.into_iter().find(|r| r.name == name).ok_or_else(|| {
            refuse_action(
                "unknown_plugin",
                format!("{name} was not installed from a source"),
            )
        })
    }

    /// The source an update fetches: the recorded one, at `new_ref` when given.
    fn update_source(
        &self,
        r: &xherd_store::PluginRecord,
        new_ref: Option<&str>,
    ) -> Result<PluginSource, ActionError> {
        let rec = Recorded::of(r);
        self.plugin_source(
            &rec.url,
            new_ref.or(rec.git_ref.as_deref()),
            rec.path.as_deref(),
        )
    }

    /// R8: fetch the recorded source of `name` (at `new_ref` when given) and
    /// show it as a preview, with the diff of its manifest against the
    /// installed one. Writes nothing.
    pub async fn plugin_update_preview(
        &self,
        name: &str,
        new_ref: Option<&str>,
    ) -> Result<UpdatePreview, ActionError> {
        let record = self.plugin_record(name).await?;
        let src = self.update_source(&record, new_ref)?;
        let (preview, new_manifest) = self
            .with_fetched(&src, |root, sha| {
                let Found::One(dir, path) = locate(root, src.path.as_deref())? else {
                    return Err(refuse_action(
                        "no_manifest",
                        "several plugins in the repository: the recorded path is gone",
                    ));
                };
                let view = self.inspect(&dir)?;
                same_name(name, &view.name)?;
                let text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap_or_default();
                Ok((
                    Preview {
                        url: src.url.clone(),
                        git_ref: src.git_ref.clone(),
                        path,
                        sha,
                        plugin: Some(view),
                        choices: Vec::new(),
                    },
                    pretty_manifest(&text),
                ))
            })
            .await?;
        let old = std::fs::read_to_string(self.paths.plugins().join(name).join(MANIFEST_FILE))
            .map(|t| pretty_manifest(&t))
            .unwrap_or_default();
        let manifest_diff = self.diff_text(&old, &new_manifest).await?;
        Ok(UpdatePreview {
            preview,
            installed: Recorded::of(&record),
            manifest_diff,
        })
    }

    /// Unified diff of two texts, through git (no diff crate in the workspace).
    async fn diff_text(&self, old: &str, new: &str) -> Result<String, ActionError> {
        let io = |e: std::io::Error| refuse_action("io", e.to_string());
        let dir = self
            .config
            .workspace_root()
            .join("repos")
            .join(format!(".xherd-plugin-diff-{}", nanos()));
        std::fs::create_dir_all(&dir).map_err(io)?;
        let (a, b) = (dir.join("installed.json"), dir.join("new.json"));
        let diff = match std::fs::write(&a, old).and_then(|()| std::fs::write(&b, new)) {
            Ok(()) => Git::new("git")
                .with_timeout(CLONE_TIMEOUT)
                .diff_files(&a, &b)
                .await
                .map_err(|e| refuse_action("io", e.to_string())),
            Err(e) => Err(io(e)),
        };
        let _ = std::fs::remove_dir_all(&dir);
        diff
    }

    /// R8, R11: replace the installed `name` by what `plugin_update_preview`
    /// showed. The source is fetched again and must digest to
    /// `expect_digest` (`changed_since_preview` otherwise, nothing written).
    /// The old folder is kept aside until the record is saved, so the swap is
    /// whole or undone. Needs a restart to load.
    pub async fn plugin_update(
        &self,
        name: &str,
        new_ref: Option<&str>,
        expect_digest: &str,
    ) -> Result<Updated, ActionError> {
        let _write = self.plugins_write.lock().await;
        let record = self.plugin_record(name).await?;
        let src = self.update_source(&record, new_ref)?;
        let plugins = self.paths.plugins();
        let (view, found_path, sha, old) = self
            .with_fetched(&src, |root, sha| {
                let Found::One(dir, found_path) = locate(root, src.path.as_deref())? else {
                    return Err(refuse_action(
                        "no_manifest",
                        "several plugins in the repository: the recorded path is gone",
                    ));
                };
                let view = self.inspect(&dir)?;
                same_name(name, &view.name)?;
                if view.digest != expect_digest {
                    return Err(refuse_action(
                        "changed_since_preview",
                        "the source changed since the preview: preview it again",
                    ));
                }
                let staging = stage_plugin(&plugins, name, &dir)?;
                let (target, aside) = (
                    plugins.join(name),
                    plugins.join(format!(".{name}.old-{}", nanos())),
                );
                let old = if target.exists() {
                    if let Err(e) = std::fs::rename(&target, &aside) {
                        let _ = std::fs::remove_dir_all(&staging);
                        return Err(refuse_action("io", format!("{name}: {e}")));
                    }
                    Some(aside)
                } else {
                    None
                };
                if let Err(e) = std::fs::rename(&staging, &target) {
                    let _ = std::fs::remove_dir_all(&staging);
                    if let Some(o) = &old {
                        let _ = std::fs::rename(o, &target);
                    }
                    return Err(refuse_action("io", format!("{name}: {e}")));
                }
                Ok((view, found_path, sha, old))
            })
            .await?;
        let now = self.clock.now();
        let mut data = record.data.clone();
        data["ref"] = serde_json::json!(src.git_ref);
        data["path"] = serde_json::json!(found_path);
        data["digest"] = serde_json::json!(view.digest);
        data["version"] = serde_json::json!(view.version);
        data["updatedAt"] = serde_json::json!(now);
        let updated = xherd_store::PluginRecord {
            sha: Some(sha.clone()),
            data,
            ..record.clone()
        };
        let recorded = async {
            self.store.upsert_plugin_record(&updated).await?;
            self.store
                .insert_event(xherd_store::Event {
                    id: xherd_core::new_id(),
                    job_id: String::new(),
                    kind: "plugin-update".into(),
                    payload: Some(serde_json::json!({
                        "name": name, "source": src.url, "sha": sha,
                        "previousSha": record.sha,
                        "digest": view.digest, "version": view.version,
                    })),
                    at: now,
                })
                .await
        }
        .await;
        if let Err(e) = recorded {
            // Put the old plugin and its record back.
            let target = plugins.join(name);
            let _ = std::fs::remove_dir_all(&target);
            if let Some(o) = &old {
                let _ = std::fs::rename(o, &target);
            }
            let _ = self.store.upsert_plugin_record(&record).await;
            return Err(refuse_action(
                "io",
                format!("cannot record the update: {e}"),
            ));
        }
        if let Some(o) = old {
            let _ = std::fs::remove_dir_all(o);
        }
        Ok(Updated {
            name: name.to_string(),
            version: view.version,
            url: src.url,
            git_ref: src.git_ref,
            path: found_path,
            sha,
            digest: view.digest,
            previous_sha: record.sha,
        })
    }

    /// The first job that still needs plugin `name`: as its runtime, or as
    /// the host of its pull request. A job that is paused, done, cancelled or
    /// in error does not count (R9).
    async fn plugin_user(&self, name: &str) -> Result<Option<String>, ActionError> {
        use xherd_core::Status;
        let jobs = self
            .store
            .list_jobs()
            .await
            .map_err(|e| refuse_action("io", e.to_string()))?;
        let cfg = self.live_config();
        let hosts = crate::host_plugins(
            &self.plugins,
            &self.paths,
            &self.config,
            &self.secrets,
            &self.xherd_bin,
        );
        for j in jobs {
            if matches!(
                j.status,
                Status::Paused | Status::Done | Status::Cancelled | Status::Error
            ) {
                continue;
            }
            let runtime = j.runtime.as_deref().unwrap_or(&cfg.default_runtime);
            let host = j.pr_url.as_deref().and_then(|u| {
                crate::actor::pick_host(
                    &hosts,
                    &cfg.git_hosts,
                    &cfg.disabled_plugins,
                    crate::git_access::remote_host(u).as_deref(),
                )
            });
            if runtime == name || host.is_some_and(|h| h.plugin.name() == name) {
                return Ok(Some(j.name));
            }
        }
        Ok(None)
    }

    /// R9, R11: delete an external plugin's folder and record. Refused
    /// (`plugin_in_use`) while a live job needs it, `builtin_plugin` for a
    /// built-in. Its settings and secrets stay unless `forget`. Allowed with
    /// the install switch off: taking code away is no risk.
    pub async fn plugin_remove(&self, name: &str, forget: bool) -> Result<Removed, ActionError> {
        let _write = self.plugins_write.lock().await;
        if self.plugins.get(name).is_some_and(|p| p.builtin()) {
            return Err(refuse_action(
                "builtin_plugin",
                format!("{name} is built in: it cannot be changed or removed"),
            ));
        }
        let unknown = || refuse_action("unknown_plugin", format!("no installed plugin {name}"));
        if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
            return Err(unknown());
        }
        let plugins = self.paths.plugins();
        let target = plugins.join(name);
        let record = self
            .store
            .plugin_records()
            .await
            .map_err(|e| refuse_action("io", e.to_string()))?
            .into_iter()
            .find(|r| r.name == name);
        if record.is_none() && !target.is_dir() {
            return Err(unknown());
        }
        if let Some(job) = self.plugin_user(name).await? {
            return Err(refuse_action(
                "plugin_in_use",
                format!("job {job} uses {name}: pause or finish it first"),
            ));
        }
        let aside = plugins.join(format!(".{name}.old-{}", nanos()));
        let moved = target.is_dir();
        if moved && let Err(e) = std::fs::rename(&target, &aside) {
            return Err(refuse_action("io", format!("{name}: {e}")));
        }
        let now = self.clock.now();
        let recorded = async {
            self.store.delete_plugin_record(name).await?;
            self.store
                .insert_event(xherd_store::Event {
                    id: xherd_core::new_id(),
                    job_id: String::new(),
                    kind: "plugin-remove".into(),
                    payload: Some(serde_json::json!({
                        "name": name,
                        "source": record.as_ref().map(|r| r.source.clone()),
                        "sha": record.as_ref().and_then(|r| r.sha.clone()),
                        "forget": forget,
                    })),
                    at: now,
                })
                .await
        }
        .await;
        if let Err(e) = recorded {
            if moved {
                let _ = std::fs::rename(&aside, &target);
            }
            if let Some(r) = &record {
                let _ = self.store.upsert_plugin_record(r).await;
            }
            return Err(refuse_action(
                "io",
                format!("cannot record the removal: {e}"),
            ));
        }
        if moved {
            let _ = std::fs::remove_dir_all(&aside);
        }
        if forget {
            self.forget_plugin(name)?;
        }
        Ok(Removed {
            name: name.to_string(),
            forgot: forget,
        })
    }

    /// Drop `name`'s settings (`config.json`) and secrets (`secrets.json`).
    fn forget_plugin(&self, name: &str) -> Result<(), ActionError> {
        let io = |e: std::io::Error| refuse_action("io", e.to_string());
        {
            let _w = self.config_write.lock().unwrap_or_else(|e| e.into_inner());
            let mut c = self.live_config();
            if c.plugin_settings.remove(name).is_some() {
                c.save(&self.paths.config()).map_err(io)?;
            }
        }
        let _w = self.secrets_write.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = self.live_secrets();
        if s.0.remove(name).is_some() {
            s.save(&self.paths.secrets()).map_err(io)?;
        }
        Ok(())
    }

    /// Check the plugin folder `dir` and describe it (R3, R5, R5a).
    pub(crate) fn inspect(&self, dir: &Path) -> Result<PluginView, ActionError> {
        let scan = scan(dir)?;
        let json = std::fs::read_to_string(dir.join(MANIFEST_FILE))
            .map_err(|e| refuse_action("bad_manifest", format!("{MANIFEST_FILE}: {e}")))?;
        let mut m: Manifest = serde_json::from_str(&json)
            .map_err(|e| refuse_action("bad_manifest", format!("{MANIFEST_FILE}: {e}")))?;
        if m.build.is_some() {
            return Err(refuse_action(
                "build_not_supported",
                "the manifest has a build step: xherd runs no code at install, so the \
                 repository must contain the built files",
            ));
        }
        if m.kind == Kind::Flow {
            return Err(refuse_action(
                "kind_not_supported",
                "flow packs cannot be installed as plugins",
            ));
        }
        // A plugin that needs a newer xherd is shown, not refused.
        let need = m.min_xherd_version.as_deref().and_then(parse_version);
        let have = parse_version(xherd_core::VERSION).expect("own version");
        let compatible = need.is_none_or(|n| n <= have);
        let min = m.min_xherd_version.clone();
        if !compatible {
            m.min_xherd_version = None;
        }
        m.validate(Some(dir))
            .map_err(|e| refuse_action("bad_manifest", e))?;
        let on_disk = self.paths.plugins().join(&m.name).exists();
        let replaces = match self.plugins.get(&m.name) {
            Some(p) if p.builtin() => Some("builtin"),
            Some(_) => Some("external"),
            None => on_disk.then_some("external"),
        };
        Ok(PluginView {
            runs: runs(&m),
            settings: m
                .settings
                .iter()
                .map(|s| SettingView {
                    key: s.key.clone(),
                    label: s.label.clone(),
                    secret: s.secret,
                    required: s.required,
                })
                .collect(),
            name: m.name,
            kind: m.kind,
            version: m.version,
            label: m.label,
            description: m.description,
            min_xherd_version: min,
            compatible,
            digest: scan.digest,
            files: scan.files,
            bytes: scan.bytes,
            replaces,
            warning: WARNING,
        })
    }
}

/// An update keeps the plugin's name: its settings, secrets and record are
/// filed under it.
fn same_name(installed: &str, fetched: &str) -> Result<(), ActionError> {
    if installed == fetched {
        return Ok(());
    }
    Err(refuse_action(
        "bad_manifest",
        format!("the source now holds the plugin {fetched}, not {installed}"),
    ))
}

/// Every argv the manifest declares: what the plugin can run (R3).
fn runs(m: &Manifest) -> Vec<Run> {
    let run = |what: String, argv: &Vec<String>| Run {
        what,
        argv: argv.clone(),
    };
    let mut out = Vec::new();
    out.extend(
        m.commands
            .iter()
            .map(|(n, a)| run(format!("command {n}"), a)),
    );
    out.extend(m.hooks.iter().map(|(n, a)| run(format!("hook {n}"), a)));
    for c in &m.checks {
        out.push(run(format!("check {}", c.name), &c.argv));
        if let Some(f) = &c.fix {
            out.push(run(format!("fix {}", c.name), f));
        }
    }
    for r in &m.requires {
        if let Some(c) = &r.check {
            out.push(run(format!("requires {} check", r.name), c));
        }
        if let Some(i) = &r.install {
            out.push(run(format!("requires {} install", r.name), i));
        }
    }
    if let Some(l) = &m.login {
        out.push(run("login".into(), l));
    }
    out
}

/// `root/rel` when every part is a real folder (not a symlink in the fetched
/// repository that points elsewhere).
fn plain_dir(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut p = root.to_path_buf();
    for part in rel.split('/').filter(|s| !s.is_empty()) {
        p.push(part);
        if !std::fs::symlink_metadata(&p).ok()?.is_dir() {
            return None;
        }
    }
    Some(p)
}

fn has_manifest(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir.join(MANIFEST_FILE)).is_ok_and(|m| m.is_file())
}

/// R2: the manifest at `path`, else at the root, else the folders of the
/// root and of `plugins/` that hold one (a single one is taken).
fn locate(root: &Path, path: Option<&str>) -> Result<Found, ActionError> {
    let none = |what: String| refuse_action("no_manifest", what);
    if let Some(p) = path {
        return match plain_dir(root, p) {
            Some(d) if has_manifest(&d) => Ok(Found::One(d, Some(p.to_string()))),
            _ => Err(none(format!("no {MANIFEST_FILE} in {p}"))),
        };
    }
    if has_manifest(root) {
        return Ok(Found::One(root.to_path_buf(), None));
    }
    let mut choices = Vec::new();
    for base in ["", "plugins"] {
        let Some(dir) = plain_dir(root, base) else {
            continue;
        };
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.') && has_manifest(&dir.join(n)))
            .collect();
        names.sort();
        for n in names {
            let rel = if base.is_empty() {
                n.clone()
            } else {
                format!("{base}/{n}")
            };
            let d = dir.join(&n);
            let loaded = std::fs::read_to_string(d.join(MANIFEST_FILE))
                .map_err(|e| e.to_string())
                .and_then(|j| serde_json::from_str::<Manifest>(&j).map_err(|e| e.to_string()))
                .and_then(|m| m.validate(Some(&d)).map(|()| m));
            choices.push(match loaded {
                Ok(m) => Choice {
                    path: rel,
                    name: m.name,
                    kind: Some(m.kind),
                    error: None,
                },
                Err(error) => Choice {
                    path: rel,
                    name: n,
                    kind: None,
                    error: Some(error),
                },
            });
        }
    }
    match choices.len() {
        0 => Err(none(format!("no {MANIFEST_FILE} found in the repository"))),
        1 => {
            let path = choices.remove(0).path;
            let dir = plain_dir(root, &path).expect("listed folder");
            Ok(Found::One(dir, Some(path)))
        }
        _ => Ok(Found::Many(choices)),
    }
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
