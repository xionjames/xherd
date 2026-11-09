//! `xherd-flow.yaml` (design §6.2). Fields not needed yet (`ui`, `inputs`,
//! `parse`, `views`, `extends`/`override`) are rejected rather than ignored, so
//! a pack that relies on them fails loudly until they are implemented.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use xherd_core::domain::{Class, Role};

use crate::template::render;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{pack}: {message}")]
pub struct PackError {
    pub pack: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pack {
    pub name: String,
    pub version: String,
    /// XAP major the pack's prompts are written for.
    pub protocol: u32,
    /// `private` (`.xherd/spec/`) or a committed folder template such as `xherd/{agent}` (R-155).
    #[serde(default = "default_spec_dir")]
    pub spec_dir: String,
    #[serde(default)]
    pub requires_committed: bool,
    /// Files that always stay in `.xherd/`, even in committed mode.
    #[serde(default)]
    pub private: Vec<String>,
    /// Runtime name → how to install the pack's commands for it.
    #[serde(default)]
    pub install: BTreeMap<String, RuntimeInstall>,
    /// Document → JSON Schema path inside the pack, checked on `gate open` (`doc_schema`).
    #[serde(default)]
    pub schemas: BTreeMap<String, String>,
    /// `resume`, `handoff`, `status` command templates.
    #[serde(default)]
    pub commands: BTreeMap<String, String>,
    pub flows: BTreeMap<String, Flow>,
}

fn default_spec_dir() -> String {
    "private".into()
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeInstall {
    /// Replaces `{prefix}` in command templates (`/xherd:`).
    pub prefix: String,
    #[serde(default)]
    pub commands: Option<InstallSet>,
    #[serde(default)]
    pub agents: Option<InstallSet>,
    /// How the runtime passes arguments to a command file (`$ARGUMENTS`).
    #[serde(default)]
    pub args: Option<String>,
    /// Enters to press after a slash command (R-020).
    #[serde(default = "one")]
    pub slash_enters: u32,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallSet {
    /// Glob inside the pack (`commands/*.md`).
    pub from: String,
    /// Destination with `{name}` (file stem) and optional `${VAR:-default}`.
    pub to: String,
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Flow {
    /// First prompt: `{prefix}`, `{key}`, `{prompt}`.
    pub start: String,
    pub stages: Vec<StageDef>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StageDef {
    pub id: String,
    pub role: Role,
    pub class: Class,
    #[serde(default)]
    pub gate: Option<GateSpec>,
    /// First matching rule gives the command sent after approval.
    #[serde(default)]
    pub next: Vec<NextRule>,
    #[serde(default)]
    pub loop_until: Option<LoopUntil>,
    /// Stage that follows once `loopUntil` holds.
    #[serde(default)]
    pub then: Option<String>,
}

/// Exactly one of `docs` or `kind`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GateSpec {
    #[serde(default)]
    pub docs: Vec<String>,
    #[serde(default)]
    pub kind: Option<GateKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GateKind {
    Commit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoopUntil {
    AllTasksDone,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NextRule {
    #[serde(default)]
    pub when: Option<Condition>,
    pub run: String,
    /// Used instead of `run` when xherd approved the gate by itself.
    #[serde(default)]
    pub auto_approved: Option<String>,
}

/// The fixed condition vocabulary (design §6.2). Written as a one-key map
/// (`{ fileExists: design.md }`), which the YAML crate does not map to enums
/// itself, hence `RawCondition`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "RawCondition")]
pub enum Condition {
    Frontmatter {
        file: String,
        key: String,
        equals: String,
    },
    FileExists(String),
    AllTasksDone(bool),
    Setting {
        key: String,
        equals: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawCondition {
    frontmatter: Option<FrontmatterCond>,
    file_exists: Option<String>,
    all_tasks_done: Option<bool>,
    setting: Option<SettingCond>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrontmatterCond {
    file: String,
    key: String,
    equals: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingCond {
    key: String,
    equals: String,
}

impl TryFrom<RawCondition> for Condition {
    type Error = String;
    fn try_from(r: RawCondition) -> Result<Condition, String> {
        let mut all = Vec::new();
        if let Some(f) = r.frontmatter {
            all.push(Condition::Frontmatter {
                file: f.file,
                key: f.key,
                equals: f.equals,
            });
        }
        if let Some(f) = r.file_exists {
            all.push(Condition::FileExists(f));
        }
        if let Some(b) = r.all_tasks_done {
            all.push(Condition::AllTasksDone(b));
        }
        if let Some(s) = r.setting {
            all.push(Condition::Setting {
                key: s.key,
                equals: s.equals,
            });
        }
        match all.len() {
            1 => Ok(all.pop().expect("one")),
            _ => Err(
                "a condition has exactly one of frontmatter, fileExists, allTasksDone, setting"
                    .into(),
            ),
        }
    }
}

/// What conditions may look at. Paths are relative to the agent's spec folder.
pub trait Facts {
    fn frontmatter(&self, file: &str, key: &str) -> Option<String>;
    fn file_exists(&self, file: &str) -> bool;
    fn all_tasks_done(&self) -> bool;
    fn setting(&self, key: &str) -> Option<String>;
}

impl Condition {
    pub fn holds(&self, f: &dyn Facts) -> bool {
        match self {
            Condition::Frontmatter { file, key, equals } => {
                f.frontmatter(file, key).as_deref() == Some(equals.as_str())
            }
            Condition::FileExists(file) => f.file_exists(file),
            Condition::AllTasksDone(want) => f.all_tasks_done() == *want,
            Condition::Setting { key, equals } => {
                f.setting(key).as_deref() == Some(equals.as_str())
            }
        }
    }
}

impl StageDef {
    /// The command template to send after this stage's gate is approved.
    pub fn next_template(&self, f: &dyn Facts, auto: bool) -> Option<&str> {
        let rule = self
            .next
            .iter()
            .find(|r| r.when.as_ref().is_none_or(|c| c.holds(f)))?;
        Some(match (&rule.auto_approved, auto) {
            (Some(a), true) => a,
            _ => &rule.run,
        })
    }

    pub fn is_commit_gate(&self) -> bool {
        self.gate
            .as_ref()
            .is_some_and(|g| g.kind == Some(GateKind::Commit))
    }

    pub fn docs(&self) -> &[String] {
        self.gate.as_ref().map_or(&[], |g| &g.docs)
    }
}

impl Flow {
    pub fn stage(&self, id: &str) -> Option<&StageDef> {
        self.stages.iter().find(|s| s.id == id)
    }

    /// The stage after `id` in declaration order.
    pub fn after(&self, id: &str) -> Option<&StageDef> {
        let i = self.stages.iter().position(|s| s.id == id)?;
        self.stages.get(i + 1)
    }
}

/// Placeholders a command template may use.
const COMMAND_VARS: [&str; 4] = ["prefix", "key", "prompt", "commit"];

fn ident_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn version_ok(v: &str) -> bool {
    let parts: Vec<&str> = v
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// A path relative to the spec folder that cannot escape it.
pub fn relative_path_ok(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && p.split('/').all(|c| !c.is_empty() && c != "..")
}

impl Pack {
    pub fn parse(yaml: &str) -> Result<Pack, PackError> {
        let p: Pack = yaml_serde::from_str(yaml).map_err(|e| PackError {
            pack: "xherd-flow.yaml".into(),
            message: e.to_string(),
        })?;
        p.validate().map_err(|message| PackError {
            pack: p.name.clone(),
            message,
        })?;
        Ok(p)
    }

    pub fn flow(&self, name: &str) -> Option<&Flow> {
        self.flows.get(name)
    }

    /// Render a command template for `runtime` (fills `{prefix}`).
    pub fn command(
        &self,
        runtime: &str,
        tpl: &str,
        vars: &[(&str, &str)],
    ) -> Result<String, String> {
        let prefix = self
            .install
            .get(runtime)
            .map(|r| r.prefix.as_str())
            .ok_or_else(|| format!("pack {} does not support runtime {runtime}", self.name))?;
        let mut all = vec![("prefix", prefix)];
        all.extend_from_slice(vars);
        render(tpl, &all)
    }

    fn validate(&self) -> Result<(), String> {
        if !ident_ok(&self.name) {
            return Err(format!("invalid pack name {:?}", self.name));
        }
        if !version_ok(&self.version) {
            return Err(format!("version {:?} is not X.Y.Z", self.version));
        }
        if self.protocol != crate::XAP_PROTOCOL {
            return Err(format!(
                "protocol {} not supported (this xherd speaks {})",
                self.protocol,
                crate::XAP_PROTOCOL
            ));
        }
        if self.spec_dir != "private" && !self.spec_dir.contains("{agent}") {
            return Err(format!(
                "specDir {:?} must be \"private\" or contain {{agent}}",
                self.spec_dir
            ));
        }
        if self.flows.is_empty() {
            return Err("no flows".into());
        }
        for f in self
            .private
            .iter()
            .chain(self.schemas.keys())
            .chain(self.schemas.values())
        {
            if !relative_path_ok(f) {
                return Err(format!("path {f:?} must be relative and stay inside"));
            }
        }
        let dummy: Vec<(&str, &str)> = COMMAND_VARS.iter().map(|v| (*v, "x")).collect();
        let check = |what: &str, tpl: &str| {
            render(tpl, &dummy)
                .map(drop)
                .map_err(|e| format!("{what}: {e}"))
        };
        for (name, tpl) in &self.commands {
            check(&format!("commands.{name}"), tpl)?;
        }
        for (rt, inst) in &self.install {
            if inst.prefix.is_empty() {
                return Err(format!("install.{rt}: empty prefix"));
            }
            for set in [&inst.commands, &inst.agents].into_iter().flatten() {
                render(&set.to, &[("name", "x")]).map_err(|e| format!("install.{rt}: {e}"))?;
            }
        }
        for (fname, flow) in &self.flows {
            if !ident_ok(fname) {
                return Err(format!("invalid flow name {fname:?}"));
            }
            check(&format!("flows.{fname}.start"), &flow.start)?;
            if flow.stages.is_empty() {
                return Err(format!("flows.{fname}: no stages"));
            }
            let mut seen = BTreeSet::new();
            for s in &flow.stages {
                let at = format!("flows.{fname}.{}", s.id);
                if !ident_ok(&s.id) {
                    return Err(format!("{at}: invalid stage id"));
                }
                if !seen.insert(s.id.as_str()) {
                    return Err(format!("{at}: duplicate stage id"));
                }
                if let Some(g) = &s.gate {
                    match (g.docs.is_empty(), g.kind) {
                        (false, None) | (true, Some(_)) => {}
                        _ => return Err(format!("{at}: gate needs exactly one of docs or kind")),
                    }
                    if let Some(d) = g.docs.iter().find(|d| !relative_path_ok(d)) {
                        return Err(format!("{at}: doc path {d:?} must be relative"));
                    }
                }
                for r in &s.next {
                    check(&at, &r.run)?;
                    if let Some(a) = &r.auto_approved {
                        check(&at, a)?;
                    }
                }
                if s.then.is_some() != s.loop_until.is_some() {
                    return Err(format!("{at}: loopUntil and then go together"));
                }
                if let Some(t) = &s.then
                    && flow.stage(t).is_none()
                {
                    return Err(format!("{at}: then refers to unknown stage {t:?}"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct F {
        design_skip: bool,
        tasks_done: bool,
    }

    impl Facts for F {
        fn frontmatter(&self, file: &str, key: &str) -> Option<String> {
            (self.design_skip && file == "spec.md" && key == "design").then(|| "skip".into())
        }
        fn file_exists(&self, _: &str) -> bool {
            false
        }
        fn all_tasks_done(&self) -> bool {
            self.tasks_done
        }
        fn setting(&self, _: &str) -> Option<String> {
            None
        }
    }

    fn feature() -> Flow {
        crate::builtin().flow("feature").unwrap().clone()
    }

    #[test]
    fn builtin_parses() {
        let p = crate::builtin();
        assert_eq!(p.name, "xherd");
        let f = feature();
        let ids: Vec<&str> = f.stages.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "spec",
                "design",
                "tasks",
                "implement",
                "review",
                "ship",
                "pr-review",
                "pr"
            ]
        );
        assert_eq!(f.stage("spec").unwrap().class, Class::Critical);
        assert_eq!(f.stage("pr").unwrap().class, Class::HumanOnly);
        // Triage and replies are human-only at every level (spec §6.3, T1).
        assert_eq!(f.stage("pr-review").unwrap().class, Class::HumanOnly);
        assert!(f.stage("implement").unwrap().is_commit_gate());
        assert_eq!(f.stage("design").unwrap().docs(), ["design.md"]);
        assert_eq!(f.after("ship").unwrap().id, "pr-review");
        assert_eq!(p.install["claude-code"].slash_enters, 1);
    }

    #[test]
    fn spec_next_depends_on_design_skip() {
        let spec = feature().stage("spec").unwrap().clone();
        assert_eq!(
            spec.next_template(&F::default(), false),
            Some("{prefix}design")
        );
        let skip = F {
            design_skip: true,
            ..F::default()
        };
        assert_eq!(spec.next_template(&skip, false), Some("{prefix}tasks"));
    }

    #[test]
    fn auto_approved_variant() {
        let ship = feature().stage("ship").unwrap().clone();
        let f = F::default();
        assert_eq!(ship.next_template(&f, false), Some("{prefix}ship"));
        assert_eq!(
            ship.next_template(&f, true),
            Some("{prefix}ship --no-ticket-update")
        );
        assert_eq!(
            feature().stage("pr").unwrap().next_template(&f, false),
            None
        );
    }

    #[test]
    fn renders_for_runtime() {
        let p = crate::builtin();
        let f = feature();
        let s = p
            .command(
                "claude-code",
                &f.start,
                &[("key", "SC-1"), ("prompt", "add login")],
            )
            .unwrap();
        assert_eq!(s, "/xherd:feature SC-1 add login");
        let c = p
            .command(
                "claude-code",
                f.stage("implement").unwrap().next[0].run.as_str(),
                &[("commit", "C01")],
            )
            .unwrap();
        assert_eq!(c, "/xherd:commit C01");
        assert!(p.command("codex", "{prefix}next", &[]).is_err());
    }

    fn with(stages: &str) -> Result<Pack, PackError> {
        Pack::parse(&format!(
            "name: t\nversion: 1.0.0\nprotocol: 1\ninstall: {{ rt: {{ prefix: '/t:' }} }}\nflows:\n  f:\n    start: '{{prefix}}go'\n    stages:\n{stages}"
        ))
    }

    #[test]
    fn validation_errors() {
        let ok = "      - { id: a, role: spec, class: critical, gate: { docs: [a.md] } }\n";
        assert!(with(ok).is_ok());
        let cases = [
            (
                "duplicate",
                "      - { id: a, role: spec, class: critical }\n      - { id: a, role: tasks, class: routine }\n",
            ),
            (
                "exactly one",
                "      - { id: a, role: spec, class: critical, gate: { docs: [a.md], kind: commit } }\n",
            ),
            (
                "exactly one",
                "      - { id: a, role: spec, class: critical, gate: {} }\n",
            ),
            (
                "relative",
                "      - { id: a, role: spec, class: critical, gate: { docs: [../a.md] } }\n",
            ),
            (
                "unknown stage",
                "      - { id: a, role: implement, class: code, loopUntil: allTasksDone, then: zz }\n",
            ),
            (
                "go together",
                "      - { id: a, role: implement, class: code, then: a }\n",
            ),
            (
                "placeholder",
                "      - { id: a, role: spec, class: critical, next: [{ run: '{prefx}x' }] }\n",
            ),
        ];
        for (want, stages) in cases {
            let e = with(stages).unwrap_err();
            assert!(e.message.contains(want), "{want}: {e}");
        }
    }

    #[test]
    fn unsupported_fields_are_rejected() {
        let e = Pack::parse(
            "name: t\nversion: 1.0.0\nprotocol: 1\nviews: {}\nflows:\n  f: { start: x, stages: [{ id: a, role: spec, class: critical }] }\n",
        )
        .unwrap_err();
        assert!(e.message.contains("views"), "{e}");
        let e = Pack::parse(
            "name: t\nversion: 1.0.0\nprotocol: 2\nflows:\n  f: { start: x, stages: [{ id: a, role: spec, class: critical }] }\n",
        )
        .unwrap_err();
        assert!(e.message.contains("protocol"), "{e}");
    }

    #[test]
    fn conditions() {
        let c: Condition = yaml_serde::from_str("allTasksDone: true").unwrap();
        assert!(!c.holds(&F::default()));
        assert!(c.holds(&F {
            tasks_done: true,
            ..F::default()
        }));
        let c: Condition = yaml_serde::from_str("fileExists: design.md").unwrap();
        assert!(!c.holds(&F::default()));
        assert!(yaml_serde::from_str::<Condition>("{}").is_err());
        assert!(
            yaml_serde::from_str::<Condition>("{ fileExists: a, allTasksDone: true }").is_err()
        );
    }
}
