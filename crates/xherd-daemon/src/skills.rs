//! The skills library (issue #141, spec R25): `<data>/skills/<name>/` holds a
//! `SKILL.md` (frontmatter `name` and `description`) and the files it uses.
//! Agents list skills by name; a job gets a copy at creation (R26, in
//! `create.rs`). Writes run under `Daemon.agents_write`, so an agent cannot
//! start using a skill while it is being deleted.

use std::path::{Component, Path, PathBuf};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use xherd_core::agent_def::SkillCopy;

use crate::{ActionError, Daemon};

/// The whole skill: every file together, as the spec says (R25).
pub const MAX_BYTES: u64 = 1024 * 1024;
const MAX_FILES: usize = 200;
const MAX_PATH: usize = 200;
/// The file that makes a folder a skill.
pub const SKILL_FILE: &str = "SKILL.md";

fn err(code: &'static str, message: impl Into<String>) -> ActionError {
    ActionError {
        code,
        message: message.into(),
    }
}

fn io(e: std::io::Error) -> ActionError {
    err("io", e.to_string())
}

/// A skill as the list shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub files: usize,
    pub bytes: u64,
    /// Over the paths and contents (what a job records, R26).
    pub sha256: String,
    /// Names of the agents that list it.
    pub used_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillFile {
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetail {
    #[serde(flatten)]
    pub info: SkillInfo,
    pub file_list: Vec<SkillFile>,
    /// The text of `SKILL.md`.
    pub text: String,
}

/// `POST /skills`: a name and either the text of `SKILL.md` or every file.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewSkill {
    pub name: String,
    pub text: Option<String>,
    pub files: Option<Vec<NewSkillFile>>,
    /// Replace a skill of that name (jobs already started keep their copy).
    #[serde(default)]
    pub replace: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewSkillFile {
    pub path: String,
    pub text: Option<String>,
    pub base64: Option<String>,
}

/// A skill read from the library: what a job copies.
#[derive(Debug)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Relative path (`/`-separated) and content, sorted by path.
    pub files: Vec<(String, Vec<u8>)>,
    pub sha256: String,
}

/// A path inside a skill: relative, plain parts, no hidden file, no `..`.
fn clean_path(p: &str) -> Result<String, ActionError> {
    let bad = |why: &str| err("bad_skill", format!("file path {p:?}: {why}"));
    if p.is_empty() || p.len() > MAX_PATH {
        return Err(bad("empty or too long"));
    }
    if p.contains('\\') || p.contains('\0') {
        return Err(bad("use plain `/` paths"));
    }
    let mut parts = Vec::new();
    for c in Path::new(p).components() {
        match c {
            Component::Normal(s) => {
                let s = s.to_str().ok_or_else(|| bad("not UTF-8"))?;
                if s.starts_with('.') {
                    return Err(bad("hidden files are not kept"));
                }
                parts.push(s.to_string());
            }
            _ => return Err(bad("must be relative, without `..`")),
        }
    }
    if parts.is_empty() {
        return Err(bad("empty"));
    }
    Ok(parts.join("/"))
}

/// Description from a `SKILL.md` text, checking the frontmatter (R25): both
/// `name` and `description` are required, and the name is the folder's.
fn read_header(name: &str, text: &str) -> Result<String, ActionError> {
    let bad = |m: String| err("bad_skill", format!("{SKILL_FILE}: {m}"));
    let fm = xherd_flow::frontmatter::parse(text)
        .map_err(bad)?
        .ok_or_else(|| {
            bad("starts with no frontmatter (`---`, name, description, `---`)".into())
        })?;
    let get = |k: &str| {
        fm.get(k)
            .map(xherd_flow::frontmatter::as_text)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let n = get("name").ok_or_else(|| bad("frontmatter has no `name`".into()))?;
    if n != name {
        return Err(bad(format!(
            "frontmatter name {n:?} is not the skill's name {name:?}"
        )));
    }
    get("description").ok_or_else(|| bad("frontmatter has no `description`".into()))
}

fn digest(files: &[(String, Vec<u8>)]) -> String {
    let mut h = Sha256::new();
    for (p, c) in files {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p.as_bytes());
        h.update((c.len() as u64).to_le_bytes());
        h.update(c);
    }
    hex::encode(h.finalize())
}

/// Every file under `dir` (sorted, relative), refusing symlinks, hidden files
/// and anything over the limits.
fn walk(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, ActionError> {
    fn go(
        base: &Path,
        dir: &Path,
        out: &mut Vec<(String, Vec<u8>)>,
        total: &mut u64,
    ) -> Result<(), ActionError> {
        for e in std::fs::read_dir(dir).map_err(io)? {
            let e = e.map_err(io)?;
            let path = e.path();
            let rel = path
                .strip_prefix(base)
                .ok()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .replace('\\', "/");
            let what = |why: &str| err("bad_skill", format!("{rel}: {why}"));
            if e.file_name().to_string_lossy().starts_with('.') {
                return Err(what("hidden files are not kept"));
            }
            let t = std::fs::symlink_metadata(&path).map_err(io)?;
            if t.file_type().is_symlink() {
                return Err(what("symbolic links are not kept"));
            }
            if t.is_dir() {
                go(base, &path, out, total)?;
            } else if t.is_file() {
                *total += t.len();
                if *total > MAX_BYTES || out.len() >= MAX_FILES {
                    return Err(err(
                        "too_large",
                        format!(
                            "a skill is at most {} KiB and {MAX_FILES} files",
                            MAX_BYTES / 1024
                        ),
                    ));
                }
                out.push((rel, std::fs::read(&path).map_err(io)?));
            } else {
                return Err(what("not a regular file"));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    go(dir, dir, &mut out, &mut 0)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Read a skill folder; `name` is the folder's.
pub fn load(root: &Path, name: &str) -> Result<Skill, ActionError> {
    let dir = root.join(name);
    if !dir.is_dir() {
        return Err(err("no_such_skill", format!("no skill named {name}")));
    }
    let files = walk(&dir)?;
    let main = files
        .iter()
        .find(|(p, _)| p == SKILL_FILE)
        .ok_or_else(|| err("bad_skill", format!("{name} has no {SKILL_FILE}")))?;
    let text = std::str::from_utf8(&main.1)
        .map_err(|_| err("bad_skill", format!("{SKILL_FILE} is not UTF-8 text")))?;
    let description = read_header(name, text)?;
    Ok(Skill {
        name: name.to_string(),
        description,
        sha256: digest(&files),
        files,
    })
}

/// Check the files of a new skill and give them as `(path, bytes)`.
fn files_of(req: &NewSkill) -> Result<Vec<(String, Vec<u8>)>, ActionError> {
    let mut out: Vec<(String, Vec<u8>)> = match (&req.text, &req.files) {
        (Some(t), None) => vec![(SKILL_FILE.to_string(), t.clone().into_bytes())],
        (None, Some(fs)) => {
            let mut v = Vec::new();
            for f in fs {
                let path = clean_path(&f.path)?;
                let bytes =
                    match (&f.text, &f.base64) {
                        (Some(t), None) => t.clone().into_bytes(),
                        (None, Some(b)) => base64::engine::general_purpose::STANDARD
                            .decode(b.trim())
                            .map_err(|_| err("bad_skill", format!("{path}: not valid base64")))?,
                        _ => {
                            return Err(err(
                                "bad_skill",
                                format!("{path}: give `text` or `base64`, one of them"),
                            ));
                        }
                    };
                v.push((path, bytes));
            }
            v
        }
        _ => {
            return Err(err(
                "bad_skill",
                "give `text` (the SKILL.md) or `files`, one of them",
            ));
        }
    };
    out.sort_by(|a, b| a.0.cmp(&b.0));
    if out.windows(2).any(|w| w[0].0 == w[1].0) {
        return Err(err("bad_skill", "a file path appears twice"));
    }
    if out.len() > MAX_FILES || out.iter().map(|f| f.1.len() as u64).sum::<u64>() > MAX_BYTES {
        return Err(err(
            "too_large",
            format!(
                "a skill is at most {} KiB and {MAX_FILES} files",
                MAX_BYTES / 1024
            ),
        ));
    }
    // A file may not sit where a folder is needed.
    for (a, _) in &out {
        if out.iter().any(|(b, _)| b.starts_with(&format!("{a}/"))) {
            return Err(err("bad_skill", format!("{a} is both a file and a folder")));
        }
    }
    Ok(out)
}

impl Daemon {
    fn skills_dir(&self) -> PathBuf {
        self.paths.skills()
    }

    /// Whether the library has a skill of that name (agent edits, R25).
    pub fn has_skill(&self, name: &str) -> bool {
        xherd_core::new::check_name(name).is_ok()
            && self.skills_dir().join(name).join(SKILL_FILE).is_file()
    }

    /// A skill from the library, for copying into a worktree (R26).
    pub fn skill(&self, name: &str) -> Result<Skill, ActionError> {
        xherd_core::new::check_name(name)
            .map_err(|_| err("no_such_skill", format!("no skill named {name}")))?;
        load(&self.skills_dir(), name)
    }

    async fn skill_users(&self) -> Result<Vec<(String, Vec<String>)>, ActionError> {
        let defs = self
            .store
            .list_agent_defs()
            .await
            .map_err(|e| err("store", e.to_string()))?;
        Ok(defs.into_iter().map(|d| (d.name, d.skills)).collect())
    }

    fn info(skill: &Skill, users: &[(String, Vec<String>)]) -> SkillInfo {
        SkillInfo {
            name: skill.name.clone(),
            description: skill.description.clone(),
            files: skill.files.len(),
            bytes: skill.files.iter().map(|f| f.1.len() as u64).sum(),
            sha256: skill.sha256.clone(),
            used_by: users
                .iter()
                .filter(|(_, s)| s.contains(&skill.name))
                .map(|(n, _)| n.clone())
                .collect(),
        }
    }

    /// Every skill in the library; a folder that is not a valid skill is
    /// left out (it cannot be used), the others are listed.
    pub async fn skills(&self) -> Result<Vec<SkillInfo>, ActionError> {
        let users = self.skill_users().await?;
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(self.skills_dir()) else {
            return Ok(out);
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if xherd_core::new::check_name(&name).is_err() {
                continue;
            }
            match load(&self.skills_dir(), &name) {
                Ok(s) => out.push(Self::info(&s, &users)),
                Err(e) => tracing::warn!(skill = %name, error = %e.message, "skill skipped"),
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub async fn skill_detail(&self, name: &str) -> Result<SkillDetail, ActionError> {
        let users = self.skill_users().await?;
        let s = self.skill(name)?;
        let text = s
            .files
            .iter()
            .find(|f| f.0 == SKILL_FILE)
            .map(|f| String::from_utf8_lossy(&f.1).into_owned())
            .unwrap_or_default();
        Ok(SkillDetail {
            info: Self::info(&s, &users),
            file_list: s
                .files
                .iter()
                .map(|(p, c)| SkillFile {
                    path: p.clone(),
                    bytes: c.len() as u64,
                })
                .collect(),
            text,
        })
    }

    /// Add a skill (R25). The folder is built beside the library and renamed
    /// into place, so a failed add leaves nothing half written.
    pub async fn create_skill(&self, req: NewSkill) -> Result<SkillDetail, ActionError> {
        xherd_core::new::check_name(&req.name)?;
        let files = files_of(&req)?;
        let main = files
            .iter()
            .find(|f| f.0 == SKILL_FILE)
            .ok_or_else(|| err("bad_skill", format!("a skill needs a {SKILL_FILE}")))?;
        let text = std::str::from_utf8(&main.1)
            .map_err(|_| err("bad_skill", format!("{SKILL_FILE} is not UTF-8 text")))?;
        read_header(&req.name, text)?;
        {
            let _w = self.agents_write.lock().await;
            let root = self.skills_dir();
            let dest = root.join(&req.name);
            if dest.exists() && !req.replace {
                return Err(err(
                    "name_taken",
                    format!("a skill named {} exists", req.name),
                ));
            }
            std::fs::create_dir_all(&root).map_err(io)?;
            let tmp = root.join(format!(".tmp-{}-{}", req.name, xherd_core::new_id()));
            let built = (|| {
                for (p, c) in &files {
                    let to = tmp.join(p);
                    if let Some(dir) = to.parent() {
                        std::fs::create_dir_all(dir).map_err(io)?;
                    }
                    xherd_core::paths::write_atomic(&to, c, Some(0o644)).map_err(io)?;
                }
                if dest.exists() {
                    std::fs::remove_dir_all(&dest).map_err(io)?;
                }
                std::fs::rename(&tmp, &dest).map_err(io)
            })();
            if built.is_err() {
                let _ = std::fs::remove_dir_all(&tmp);
            }
            built?;
        }
        self.skill_detail(&req.name).await
    }

    /// Remove a skill; refused while an agent lists it (R25).
    pub async fn delete_skill(&self, name: &str) -> Result<(), ActionError> {
        let _w = self.agents_write.lock().await;
        xherd_core::new::check_name(name)
            .map_err(|_| err("no_such_skill", format!("no skill named {name}")))?;
        let dir = self.skills_dir().join(name);
        if !dir.is_dir() {
            return Err(err("no_such_skill", format!("no skill named {name}")));
        }
        let users = self.skill_users().await?;
        let used: Vec<&str> = users
            .iter()
            .filter(|(_, s)| s.iter().any(|x| x == name))
            .map(|(n, _)| n.as_str())
            .collect();
        if !used.is_empty() {
            return Err(err(
                "skill_in_use",
                format!("agent {} lists {name}", used.join(", ")),
            ));
        }
        std::fs::remove_dir_all(dir).map_err(io)
    }
}

pub(crate) async fn install(
    root: &Path,
    workdir: &Path,
    rt_name: &str,
    target: Option<&xherd_plugin::runtime::SkillTarget>,
    wanted: &[(String, Option<String>)],
) -> Result<Vec<SkillCopy>, ActionError> {
    let mut out = Vec::new();
    for (name, expected) in wanted {
        let skip = |sha: &str, why: String| SkillCopy {
            name: name.clone(),
            sha256: sha.to_string(),
            installed: false,
            skipped: Some(why),
        };
        let skill = match load(root, name) {
            Ok(s) => s,
            Err(e) => {
                out.push(skip(
                    "",
                    format!("not in the skills library: {}", e.message),
                ));
                continue;
            }
        };
        if expected.as_ref().is_some_and(|e| *e != skill.sha256) {
            out.push(skip(
                &skill.sha256,
                "the library copy changed since the job started".into(),
            ));
            continue;
        }
        let Some(target) = target else {
            out.push(skip(
                &skill.sha256,
                format!("{rt_name} has no skill folder"),
            ));
            continue;
        };
        let dir = workdir.join(&target.dir).join(name);
        if dir.exists() {
            out.push(skip(&skill.sha256, "the repository has its own".into()));
            continue;
        }
        let io =
            |p: &Path, e: std::io::Error| err("skill_install", format!("{}: {e}", p.display()));
        let mut files: Vec<(PathBuf, &[u8])> = skill
            .files
            .iter()
            .map(|(p, b)| (dir.join(p), b.as_slice()))
            .collect();
        files.push((dir.join(".gitignore"), b"*\n"));
        let copied = async {
            xherd_core::runner::mkdir(&dir, DIR_MODE)
                .await
                .map_err(|e| io(&dir, e))?;
            for (path, bytes) in &files {
                if let Some(parent) = path.parent().filter(|p| *p != dir) {
                    xherd_core::runner::mkdir(parent, DIR_MODE)
                        .await
                        .map_err(|e| io(parent, e))?;
                }
                xherd_core::runner::write_file(path, bytes, None)
                    .await
                    .map_err(|e| io(path, e))?;
            }
            Ok::<(), ActionError>(())
        }
        .await;
        if let Err(e) = copied {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
        out.push(SkillCopy {
            name: name.clone(),
            sha256: skill.sha256,
            installed: true,
            skipped: None,
        });
    }
    Ok(out)
}

/// Mode of the folders xherd creates in a worktree (shared with the agents).
const DIR_MODE: u32 = 0o2775;

/// Take the skills a switch leaves behind out of the worktree (D26, R39):
/// only folders xherd copied (`installed`) that still hold its `.gitignore`
/// marker; a folder the repository owns is never touched. Removal runs as
/// the agents' user. Failures are only logged.
pub(crate) async fn remove(
    workdir: &Path,
    target: Option<&xherd_plugin::runtime::SkillTarget>,
    copies: &[SkillCopy],
) {
    let Some(target) = target else { return };
    for s in copies.iter().filter(|s| s.installed) {
        let dir = workdir.join(&target.dir).join(&s.name);
        let marked = matches!(
            xherd_core::runner::read_file(&dir.join(".gitignore")).await,
            Ok(Some(b)) if b == b"*\n"
        );
        if !marked {
            continue;
        }
        let cmd = xherd_core::exec::Cmd::new(["rm", "-rf", "--"])
            .timeout(std::time::Duration::from_secs(30))
            .as_agent();
        let mut cmd = cmd;
        cmd.argv.push(dir.to_string_lossy().into_owned());
        match xherd_core::exec::run(cmd).await {
            Ok(o) if o.code == Some(0) => {}
            Ok(o) => tracing::warn!(skill = %s.name, stderr = %o.stderr, "skill not removed"),
            Err(e) => tracing::warn!(skill = %s.name, error = %e, "skill not removed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_the_skill() {
        assert_eq!(clean_path("a/b.md").unwrap(), "a/b.md");
        for p in ["", "/etc/x", "../x", "a/../x", ".git/x", "a/.env", "a\\b"] {
            assert_eq!(clean_path(p).unwrap_err().code, "bad_skill", "{p}");
        }
    }

    #[test]
    fn the_header_needs_name_and_description() {
        let ok = "---\nname: lint\ndescription: Runs the linter\n---\nBody";
        assert_eq!(read_header("lint", ok).unwrap(), "Runs the linter");
        for t in [
            "Body only",
            "---\nname: lint\n---\n",
            "---\ndescription: x\n---\n",
            "---\nname: other\ndescription: x\n---\n",
            "---\nname: lint\ndescription: \"\"\n---\n",
        ] {
            assert_eq!(read_header("lint", t).unwrap_err().code, "bad_skill", "{t}");
        }
    }

    #[test]
    fn files_are_checked_before_anything_is_written() {
        let req = |files: Vec<NewSkillFile>| NewSkill {
            name: "s".into(),
            text: None,
            files: Some(files),
            replace: false,
        };
        let f = |p: &str| NewSkillFile {
            path: p.into(),
            text: Some("x".into()),
            base64: None,
        };
        assert!(files_of(&req(vec![f("SKILL.md"), f("ref/a.md")])).is_ok());
        assert!(files_of(&req(vec![f("SKILL.md"), f("SKILL.md")])).is_err());
        assert!(files_of(&req(vec![f("a"), f("a/b")])).is_err());
        let big = NewSkillFile {
            path: "big".into(),
            text: Some("x".repeat(MAX_BYTES as usize + 1)),
            base64: None,
        };
        assert_eq!(files_of(&req(vec![big])).unwrap_err().code, "too_large");
        let both = NewSkillFile {
            path: "x".into(),
            text: Some("x".into()),
            base64: Some("eA==".into()),
        };
        assert!(files_of(&req(vec![both])).is_err());
    }
}
