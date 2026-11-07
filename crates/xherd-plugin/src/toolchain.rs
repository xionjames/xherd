//! Toolchain plugins (R-531, R-532, R-652; design §8.6): which version files a
//! tool reads, how to tell whether a version is installed (`resolve`), how to
//! install it, and the environment it needs. The daemon turns a worktree's
//! version files into a [`Plan`]; `xherd toolchain exec` carries it out in the
//! agent's tmux window and then starts the runtime with the resulting `PATH`.
//! No global "current version" is ever switched.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::Manifest;

/// `data.toolchain` of a toolchain plugin.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Toolchain {
    pub tools: Vec<Tool>,
    /// Variables of the session passed to `resolve` and `install` on top of
    /// the usual allowlist (e.g. `NVM_DIR`).
    #[serde(default)]
    pub pass_env: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tool {
    pub name: String,
    /// Read in order; the first file that names a version wins.
    pub files: Vec<VersionFile>,
    /// argv; exit 0 = installed. An absolute directory on the last line of
    /// stdout goes on `PATH`. `{v}` is replaced by the version (whole
    /// argument values only, so scripts take it as `"$1"`).
    #[serde(default)]
    pub resolve: Option<Vec<String>>,
    #[serde(default)]
    pub install: Option<Vec<String>>,
    /// Session variables; values may use `{v}`, `{bin}` (the resolved
    /// directory) and `{home}` (its parent).
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VersionFile {
    /// Worktree-relative.
    pub path: String,
    pub format: Format,
    /// `sdkmanrc`: the key to read (default: the tool name).
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// First line that is not empty or a `#` comment.
    Plain,
    /// `key=value` lines.
    Sdkmanrc,
    /// `requires-python = "…"`.
    Pyproject,
    /// `toolchain goX.Y.Z`, else `go X.Y`.
    GoMod,
    /// `channel = "…"` (TOML), else plain.
    RustToolchain,
}

impl Toolchain {
    pub fn of(m: &Manifest) -> Result<Toolchain, String> {
        let v = m
            .data
            .get("toolchain")
            .cloned()
            .ok_or_else(|| format!("toolchain plugin {} has no data.toolchain", m.name))?;
        serde_json::from_value(v).map_err(|e| format!("toolchain {}: {e}", m.name))
    }
}

static SDKMAN_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*([A-Za-z0-9_-]+)\s*=\s*(\S+)\s*$").unwrap());
static REQUIRES_PYTHON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^\s*requires-python\s*=\s*["']([^"']+)["']"#).unwrap());
static GO_TOOLCHAIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*toolchain\s+go(\S+)\s*$").unwrap());
static GO_DIRECTIVE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*go\s+(\S+)\s*$").unwrap());
static RUST_CHANNEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^\s*channel\s*=\s*["']([^"']+)["']"#).unwrap());
/// What a version may look like (`21.0.2-tem`, `lts/*`, `>=3.11,<3.13`,
/// `stable`). Anything else (spaces removed first) is ignored, so a version
/// file cannot smuggle arguments or newlines into a command.
static VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9<>=~^!][A-Za-z0-9._+*/~=<>^,!-]{0,63}$").unwrap());

/// The version a file's text names, if any.
pub fn read_version(text: &str, format: Format, key: &str) -> Option<String> {
    let plain = || {
        text.lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
    };
    let raw = match format {
        Format::Plain => plain(),
        Format::Sdkmanrc => text.lines().find_map(|l| {
            let c = SDKMAN_LINE.captures(l)?;
            (&c[1] == key).then(|| c[2].to_string())
        }),
        Format::Pyproject => REQUIRES_PYTHON.captures(text).map(|c| c[1].to_string()),
        Format::GoMod => GO_TOOLCHAIN
            .captures(text)
            .or_else(|| GO_DIRECTIVE.captures(text))
            .map(|c| c[1].to_string()),
        Format::RustToolchain => match RUST_CHANNEL.captures(text) {
            Some(c) => Some(c[1].to_string()),
            None if text.contains('[') => None,
            None => plain(),
        },
    }?;
    let v: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    VERSION.is_match(&v).then_some(v)
}

/// One tool to provide for a worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub plugin: String,
    pub tool: String,
    pub version: String,
    /// The version file it came from.
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pass_env: Vec<String>,
}

/// What `xherd toolchain exec` does before starting the runtime (written to
/// `.xherd/toolchain.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub steps: Vec<Step>,
    /// Install missing versions (container); native installs only resolve (R-653).
    pub install: bool,
}

/// Worktree-relative file where the plan is written.
pub const PLAN_FILE: &str = ".xherd/toolchain.json";

fn subst(argv: &[String], v: &str) -> Vec<String> {
    argv.iter().map(|a| a.replace("{v}", v)).collect()
}

/// The plan for a worktree: for each tool of each toolchain plugin, the first
/// version file that names a valid version. When two plugins claim a tool,
/// the first (registry order) wins (profiles choose later, R-532).
pub fn plan(worktree: &Path, toolchains: &[(String, Toolchain)], install: bool) -> Plan {
    let mut steps: Vec<Step> = Vec::new();
    for (plugin, tc) in toolchains {
        for tool in &tc.tools {
            if steps.iter().any(|s| s.tool == tool.name) {
                continue;
            }
            let found = tool.files.iter().find_map(|f| {
                let text = std::fs::read_to_string(worktree.join(&f.path)).ok()?;
                let key = f.key.as_deref().unwrap_or(&tool.name);
                read_version(&text, f.format, key).map(|v| (f.path.clone(), v))
            });
            let Some((file, v)) = found else { continue };
            steps.push(Step {
                plugin: plugin.clone(),
                tool: tool.name.clone(),
                file,
                resolve: tool.resolve.as_deref().map(|a| subst(a, &v)),
                install: tool.install.as_deref().map(|a| subst(a, &v)),
                env: tool
                    .env
                    .iter()
                    .map(|(k, val)| (k.clone(), val.replace("{v}", &v)))
                    .collect(),
                pass_env: tc.pass_env.clone(),
                version: v,
            });
        }
    }
    Plan { steps, install }
}

/// A step's session variables once its directory is known.
pub fn step_env(step: &Step, bin: Option<&Path>) -> Vec<(String, String)> {
    let bin_s = bin.map(|b| b.to_string_lossy().into_owned());
    let home_s = bin
        .and_then(Path::parent)
        .map(|h| h.to_string_lossy().into_owned());
    step.env
        .iter()
        .filter(|(_, v)| {
            (!v.contains("{bin}") || bin_s.is_some()) && (!v.contains("{home}") || home_s.is_some())
        })
        .map(|(k, v)| {
            let v = v
                .replace("{bin}", bin_s.as_deref().unwrap_or(""))
                .replace("{home}", home_s.as_deref().unwrap_or(""));
            (k.clone(), v)
        })
        .collect()
}

/// The directory a `resolve` command printed, if its last line is one.
pub fn resolved_dir(stdout: &str) -> Option<std::path::PathBuf> {
    let last = stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())?;
    let p = Path::new(last);
    (p.is_absolute() && p.is_dir()).then(|| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_files() {
        assert_eq!(
            read_version("# c\n\nv20.11.1\n", Format::Plain, ""),
            Some("v20.11.1".into())
        );
        assert_eq!(
            read_version("lts/*\n", Format::Plain, ""),
            Some("lts/*".into())
        );
        let rc = "# sdkman\njava=21.0.2-tem\nmaven = 3.9.6\n";
        assert_eq!(
            read_version(rc, Format::Sdkmanrc, "java"),
            Some("21.0.2-tem".into())
        );
        assert_eq!(
            read_version(rc, Format::Sdkmanrc, "maven"),
            Some("3.9.6".into())
        );
        assert_eq!(read_version(rc, Format::Sdkmanrc, "gradle"), None);
        let py = "[project]\nname = \"x\"\nrequires-python = \">=3.11, <3.13\"\n";
        assert_eq!(
            read_version(py, Format::Pyproject, ""),
            Some(">=3.11,<3.13".into())
        );
        let gm = "module x\n\ngo 1.22\n\ntoolchain go1.22.4\n";
        assert_eq!(read_version(gm, Format::GoMod, ""), Some("1.22.4".into()));
        assert_eq!(
            read_version("module x\ngo 1.21.0\n", Format::GoMod, ""),
            Some("1.21.0".into())
        );
        let rt = "[toolchain]\nchannel = \"1.79.0\"\ncomponents = [\"clippy\"]\n";
        assert_eq!(
            read_version(rt, Format::RustToolchain, ""),
            Some("1.79.0".into())
        );
        assert_eq!(
            read_version("stable\n", Format::RustToolchain, ""),
            Some("stable".into())
        );
        assert_eq!(
            read_version("[toolchain]\n", Format::RustToolchain, ""),
            None
        );
    }

    #[test]
    fn hostile_versions_are_ignored() {
        assert_eq!(read_version("$(rm -rf /)\n", Format::Plain, ""), None);
        assert_eq!(read_version("-rf\n", Format::Plain, ""), None);
        assert_eq!(read_version("a;b\n", Format::Plain, ""), None);
        assert_eq!(read_version("1\"2\n", Format::Plain, ""), None);
    }

    fn tc() -> Vec<(String, Toolchain)> {
        let node: Toolchain = serde_json::from_value(serde_json::json!({
            "passEnv": ["NVM_DIR"],
            "tools": [{
                "name": "node",
                "files": [{"path": ".nvmrc", "format": "plain"}, {"path": ".node-version", "format": "plain"}],
                "resolve": ["sh", "-c", "echo \"$1\"", "_", "{v}"],
                "install": ["nvm-install", "{v}"],
                "env": {"NODE_HOME": "{home}"}
            }]
        }))
        .unwrap();
        let other: Toolchain = serde_json::from_value(serde_json::json!({
            "tools": [{"name": "node", "files": [{"path": ".nvmrc", "format": "plain"}]},
                      {"name": "go", "files": [{"path": "go.mod", "format": "go-mod"}], "env": {"GOTOOLCHAIN": "auto"}}]
        }))
        .unwrap();
        vec![("nvm".into(), node), ("other".into(), other)]
    }

    #[test]
    fn plan_reads_the_first_file_and_first_plugin() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(".node-version"), "20\n").unwrap();
        std::fs::write(d.path().join("go.mod"), "module x\ngo 1.22.1\n").unwrap();
        let p = plan(d.path(), &tc(), true);
        assert!(p.install);
        assert_eq!(p.steps.len(), 2);
        let n = &p.steps[0];
        assert_eq!(
            (n.plugin.as_str(), n.tool.as_str(), n.version.as_str()),
            ("nvm", "node", "20")
        );
        assert_eq!(n.file, ".node-version");
        assert_eq!(n.resolve.as_ref().unwrap()[4], "20");
        assert_eq!(n.install.as_ref().unwrap(), &["nvm-install", "20"]);
        assert_eq!(n.pass_env, ["NVM_DIR"]);
        assert_eq!(p.steps[1].tool, "go");

        std::fs::write(d.path().join(".nvmrc"), "lts/iron\n").unwrap();
        assert_eq!(plan(d.path(), &tc(), false).steps[0].version, "lts/iron");
        let empty = tempfile::tempdir().unwrap();
        assert!(plan(empty.path(), &tc(), false).steps.is_empty());
    }

    #[test]
    fn env_uses_the_resolved_dir() {
        let mut s = plan(Path::new("/nonexistent"), &tc(), false);
        assert!(s.steps.is_empty());
        let step = Step {
            plugin: "p".into(),
            tool: "java".into(),
            version: "21".into(),
            file: ".sdkmanrc".into(),
            resolve: None,
            install: None,
            env: BTreeMap::from([
                ("JAVA_HOME".into(), "{home}".into()),
                ("X".into(), "v{v}".into()),
            ]),
            pass_env: vec![],
        };
        s.steps.push(step.clone());
        assert_eq!(
            step_env(&step, Some(Path::new("/o/java/21/bin"))),
            [
                ("JAVA_HOME".to_string(), "/o/java/21".to_string()),
                ("X".into(), "v{v}".into())
            ]
        );
        // No directory: variables that need one are left out.
        assert_eq!(step_env(&step, None), [("X".into(), "v{v}".into())]);
    }

    #[test]
    fn resolved_dir_needs_an_absolute_existing_dir() {
        let d = tempfile::tempdir().unwrap();
        let s = format!("noise\n{}\n", d.path().display());
        assert_eq!(resolved_dir(&s), Some(d.path().to_path_buf()));
        assert_eq!(resolved_dir("rustc 1.79.0\n"), None);
        assert_eq!(resolved_dir("/nonexistent/xherd\n"), None);
    }
}
