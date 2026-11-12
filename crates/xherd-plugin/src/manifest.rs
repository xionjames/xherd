//! `xherd-plugin.json` (R-502).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const MANIFEST_FILE: &str = "xherd-plugin.json";

/// Name of the check that says whether a plugin with a `login` is logged in.
pub const LOGIN_CHECK: &str = "login";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Runtime,
    Tracker,
    Host,
    Flow,
    Notifier,
    Toolchain,
    Panel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setting {
    pub key: String,
    #[serde(default)]
    pub label: String,
    /// `string`, `number`, `boolean`, `select`… (validated by the UI form).
    #[serde(rename = "type", default = "default_setting_type")]
    pub kind: String,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub required: bool,
}

fn default_setting_type() -> String {
    "string".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    pub argv: Vec<String>,
    /// Command that fixes a failed check (offered in Setup checks).
    #[serde(default)]
    pub fix: Option<Vec<String>>,
    #[serde(default)]
    pub optional: bool,
}

/// A command-line tool the plugin needs (R-691). It becomes a check named
/// `requires:<name>`; when it fails, the container offers `install` from the
/// UI (in a terminal, into the agents' home) and a native run shows
/// `instructions` for its OS (xherd never installs on the host, R-653).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Requirement {
    /// The binary's name.
    pub name: String,
    /// How to tell it is there; default: `command -v <name>`.
    #[serde(default)]
    pub check: Option<Vec<String>>,
    /// argv that installs it, run in the container only.
    #[serde(default)]
    pub install: Option<Vec<String>>,
    /// Text by OS (`macos`, `linux`, `any`) for a native run.
    #[serde(default)]
    pub instructions: BTreeMap<String, String>,
    /// A missing optional tool is reported but never blocks.
    #[serde(default)]
    pub optional: bool,
}

impl Requirement {
    /// The check this requirement stands for.
    pub fn as_check(&self) -> Check {
        Check {
            name: format!("{REQUIRES_PREFIX}{}", self.name),
            argv: self.check.clone().unwrap_or_else(|| {
                vec![
                    "sh".into(),
                    "-c".into(),
                    r#"command -v "$1" >/dev/null 2>&1 || { echo "$1 not found" >&2; exit 1; }"#
                        .into(),
                    "_".into(),
                    self.name.clone(),
                ]
            }),
            fix: None,
            optional: self.optional,
        }
    }

    /// The instructions for `os` (`std::env::consts::OS`), else `any`.
    pub fn instructions_for(&self, os: &str) -> Option<&str> {
        self.instructions
            .get(os)
            .or_else(|| self.instructions.get("any"))
            .map(String::as_str)
    }
}

/// Prefix of the check name a [`Requirement`] makes.
pub const REQUIRES_PREFIX: &str = "requires:";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub name: String,
    pub kind: Kind,
    pub version: String,
    #[serde(rename = "min_xherd_version", alias = "minXherdVersion", default)]
    pub min_xherd_version: Option<String>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub settings: Vec<Setting>,
    /// Kind-specific data (runtime drivers, tracker key pattern, …).
    #[serde(default)]
    pub data: Map<String, Value>,
    /// Prompt fragments by name.
    #[serde(default)]
    pub instructions: BTreeMap<String, String>,
    /// Command name → argv.
    #[serde(default)]
    pub commands: BTreeMap<String, Vec<String>>,
    /// Event name → argv; output ignored.
    #[serde(default)]
    pub hooks: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub checks: Vec<Check>,
    /// Tools the plugin needs (R-691).
    #[serde(default)]
    pub requires: Vec<Requirement>,
    /// Interactive login (runs in a terminal the UI shows). Its status is a
    /// check named [`LOGIN_CHECK`].
    #[serde(default)]
    pub login: Option<Vec<String>>,
    /// Install-time build argv.
    #[serde(default)]
    pub build: Option<Vec<String>>,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

fn default_timeout() -> u64 {
    60
}

static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]{0,63}$").unwrap());

/// `X.Y.Z` (pre-release and build suffixes ignored) as a comparable tuple.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = (
        it.next()??,
        it.next().unwrap_or(Some(0))?,
        it.next().unwrap_or(Some(0))?,
    );
    it.next().is_none().then_some(v)
}

impl Manifest {
    /// The checks the plugin declares and those its requirements make.
    pub fn all_checks(&self) -> Vec<Check> {
        self.checks
            .iter()
            .cloned()
            .chain(self.requires.iter().map(Requirement::as_check))
            .collect()
    }

    pub fn parse(json: &str) -> Result<Manifest, String> {
        let m: Manifest =
            serde_json::from_str(json).map_err(|e| format!("{MANIFEST_FILE}: {e}"))?;
        m.validate(None)?;
        Ok(m)
    }

    pub fn load(dir: &Path) -> Result<Manifest, String> {
        let file = dir.join(MANIFEST_FILE);
        let json =
            std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let m: Manifest =
            serde_json::from_str(&json).map_err(|e| format!("{}: {e}", file.display()))?;
        m.validate(Some(dir))?;
        Ok(m)
    }

    /// Structural checks. With `root`, `./x` programs must resolve inside it.
    pub fn validate(&self, root: Option<&Path>) -> Result<(), String> {
        if !NAME.is_match(&self.name) {
            return Err(format!("name {:?} must match {}", self.name, NAME.as_str()));
        }
        if parse_version(&self.version).is_none() {
            return Err(format!("version {:?} is not X.Y.Z", self.version));
        }
        if let Some(min) = &self.min_xherd_version {
            let need = parse_version(min)
                .ok_or_else(|| format!("min_xherd_version {min:?} is not X.Y.Z"))?;
            let have = parse_version(xherd_core::VERSION).expect("own version");
            if need > have {
                return Err(format!(
                    "requires xherd {min} or newer (this is {})",
                    xherd_core::VERSION
                ));
            }
        }
        if self.timeout_seconds == 0 {
            return Err("timeoutSeconds must be > 0".into());
        }
        let mut keys = std::collections::HashSet::new();
        for s in &self.settings {
            if !keys.insert(&s.key) {
                return Err(format!("setting {:?} is declared twice", s.key));
            }
        }
        let mut seen = std::collections::HashSet::new();
        for r in &self.requires {
            let plain = !r.name.is_empty()
                && r.name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
            if !plain || !seen.insert(&r.name) {
                return Err(format!("requires {:?}: not a plain, unique name", r.name));
            }
            if let Some(i) = &r.install
                && i.first().is_none_or(|p| p.starts_with("./"))
            {
                return Err(format!(
                    "requires {}: install must name a program on PATH",
                    r.name
                ));
            }
            for t in r.instructions.values().chain(r.install.iter().flatten()) {
                if t.chars().any(|c| c.is_control() && c != '\n') {
                    return Err(format!("requires {}: control character", r.name));
                }
            }
        }
        let argvs = self
            .commands
            .iter()
            .map(|(n, a)| (format!("command {n}"), a))
            .chain(self.hooks.iter().map(|(n, a)| (format!("hook {n}"), a)))
            .chain(
                self.checks
                    .iter()
                    .map(|c| (format!("check {}", c.name), &c.argv)),
            )
            .chain(self.requires.iter().filter_map(|r| {
                r.check
                    .as_ref()
                    .map(|c| (format!("requires {}", r.name), c))
            }))
            .chain(
                self.checks
                    .iter()
                    .filter_map(|c| c.fix.as_ref().map(|f| (format!("fix {}", c.name), f))),
            )
            .chain(self.build.iter().map(|b| ("build".to_string(), b)));
        for (what, argv) in argvs {
            let Some(prog) = argv.first() else {
                return Err(format!("{what}: empty argv"));
            };
            if let Some(rel) = prog.strip_prefix("./") {
                if Path::new(rel)
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
                {
                    return Err(format!(
                        "{what}: {prog:?} must stay inside the plugin folder"
                    ));
                }
                if let Some(root) = root
                    && !root.join(rel).is_file()
                {
                    return Err(format!("{what}: {prog:?} not found in the plugin folder"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = r#"{
        "name": "github", "kind": "host", "version": "1.2.0", "min_xherd_version": "0.1.0",
        "settings": [{"key": "token", "secret": true}, {"key": "draft", "type": "boolean", "default": true}],
        "commands": {"pr-for-branch": ["gh", "pr", "view"]},
        "checks": [{"name": "gh", "argv": ["gh", "--version"]}]
    }"#;

    #[test]
    fn parses_and_defaults() {
        let m = Manifest::parse(OK).unwrap();
        assert_eq!(m.kind, Kind::Host);
        assert_eq!(m.timeout_seconds, 60);
        assert!(m.settings[0].secret);
        assert_eq!(m.settings[1].default, Some(Value::Bool(true)));
    }

    #[test]
    fn rejects_bad_manifests() {
        let bad = |patch: &str| {
            let mut v: Value = serde_json::from_str(OK).unwrap();
            let p: Value = serde_json::from_str(patch).unwrap();
            for (k, val) in p.as_object().unwrap() {
                v[k] = val.clone();
            }
            Manifest::parse(&v.to_string()).unwrap_err()
        };
        assert!(bad(r#"{"name": "Bad Name"}"#).contains("name"));
        assert!(bad(r#"{"version": "one"}"#).contains("version"));
        assert!(bad(r#"{"min_xherd_version": "999.0.0"}"#).contains("requires xherd"));
        assert!(bad(r#"{"commands": {"x": []}}"#).contains("empty argv"));
        assert!(bad(r#"{"commands": {"x": ["./../evil"]}}"#).contains("inside"));
        assert!(bad(r#"{"kind": "spaceship"}"#).contains("unknown variant"));
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("v1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("1.2.3-rc.1"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.x"), None);
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
    }

    #[test]
    fn requirements_become_checks_and_say_how_to_install() {
        let m = Manifest::parse(
            r#"{"name": "p", "kind": "host", "version": "0.1.0",
                "checks": [{"name": "c", "argv": ["true"]}],
                "requires": [{"name": "glab", "optional": true,
                    "install": ["sh", "-c", "echo hi"],
                    "instructions": {"macos": "brew install glab", "any": "see the docs"}}]}"#,
        )
        .unwrap();
        let names: Vec<_> = m.all_checks().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["c", "requires:glab"]);
        let r = &m.requires[0];
        assert!(r.as_check().optional);
        assert_eq!(r.as_check().argv.last().map(String::as_str), Some("glab"));
        assert_eq!(r.instructions_for("macos"), Some("brew install glab"));
        assert_eq!(r.instructions_for("linux"), Some("see the docs"));
    }

    #[test]
    fn bad_requirements_are_refused() {
        for bad in [
            r#"{"name": "a b"}"#,
            r#"{"name": "x", "install": ["./local.sh"]}"#,
            r#"{"name": "x", "install": []}"#,
            r#"{"name": "x", "instructions": {"any": "a\u0007b"}}"#,
            r#"{"name": "x", "unknown": 1}"#,
        ] {
            let m = format!(
                r#"{{"name": "p", "kind": "host", "version": "0.1.0", "requires": [{bad}]}}"#
            );
            assert!(Manifest::parse(&m).is_err(), "{bad}");
        }
        let twice = r#"{"name": "p", "kind": "host", "version": "0.1.0",
            "requires": [{"name": "x"}, {"name": "x"}]}"#;
        assert!(Manifest::parse(twice).is_err());
    }
}
