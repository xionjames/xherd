//! Runtime plugins, terminal driver part (R-511): the launch argv, permission
//! modes, and the hook file that makes the runtime call `xherd hook <event>`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::Manifest;

/// Tool-permission mode used until agents can choose one: file edits in the
/// worktree are accepted, anything else (shell commands that change things)
/// still asks and shows as `blocked`. `ask` stays available (spec §13 T1).
pub const DEFAULT_PERMISSION_MODE: &str = "edits";

/// Every runtime must offer this mode, the strictest.
pub const STRICT_PERMISSION_MODE: &str = "ask";

/// The permission modes shared by every runtime (design §4.1): `ask` asks
/// before every tool use that needs permission, `edits` accepts file edits,
/// `auto` lets the runtime's own safety checks decide, `all` asks nothing.
/// A runtime maps each mode it offers to explicit flags, so its behaviour
/// never follows a default the runtime may change; `ask` is required.
pub const PERMISSION_MODES: &[&str] = &["ask", "edits", "auto", "all"];

#[test]
fn modes_match_the_core_vocabulary() {
    use xherd_core::permission::PermissionMode;
    let core: Vec<&str> = PermissionMode::ALL.iter().map(|m| m.as_str()).collect();
    assert_eq!(core, PERMISSION_MODES);
    assert_eq!(PermissionMode::default().as_str(), DEFAULT_PERMISSION_MODE);
}

/// Appended to every hook command xherd writes, so its entries can be found
/// and replaced without touching the user's own hooks (R-005b).
pub const HOOK_TAG: &str = "# xherd";

/// `data.terminal` of a runtime plugin.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Terminal {
    pub launch: Vec<String>,
    /// Mode name → extra launch arguments (and variables).
    #[serde(default)]
    pub permission_modes: BTreeMap<String, ModeArgs>,
    #[serde(default)]
    pub hooks: Option<HookFile>,
    #[serde(default)]
    pub trust: Option<Trust>,
    /// How to continue the runtime's last conversation (R-027).
    #[serde(default)]
    pub resume: Option<Resume>,
    /// How to read token usage from the transcript (R-1205). The transcript
    /// is found through `resume`, so it needs one.
    #[serde(default)]
    pub usage: Option<Usage>,
    /// A one-shot, read-only run that answers a question (TODO condition
    /// checks, spec §16.2); runtimes without it cannot check conditions.
    #[serde(default)]
    pub check: Option<Check>,
    /// Launch arguments that give a new session its first prompt, `{prompt}`
    /// once; the runtime submits it itself when ready. Without them xherd
    /// types it at `session-start` (a runtime whose input is not ready then,
    /// like opencode's, would lose the text).
    #[serde(default)]
    pub prompt_args: Vec<String>,
}

impl Terminal {
    /// `prompt_args` with the prompt; `None` when the runtime has none.
    pub fn prompt_argv(args: &[String], prompt: &str) -> Option<Vec<String>> {
        (!args.is_empty()).then(|| args.iter().map(|a| a.replace("{prompt}", prompt)).collect())
    }
}

/// What a permission mode adds to the launch: arguments, and variables for a
/// runtime that takes its permissions from the environment (opencode's
/// `OPENCODE_PERMISSION`). Written as a list (arguments only) or as
/// `{"args": [...], "env": {...}}`.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(from = "ModeSpec")]
pub struct ModeArgs {
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ModeSpec {
    Args(Vec<String>),
    Full {
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
}

impl From<ModeSpec> for ModeArgs {
    fn from(s: ModeSpec) -> ModeArgs {
        match s {
            ModeSpec::Args(args) => ModeArgs {
                args,
                env: BTreeMap::new(),
            },
            ModeSpec::Full { args, env } => ModeArgs { args, env },
        }
    }
}

/// How to run the runtime once, headless, read-only (`data.terminal.check`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Check {
    /// The command. `{prompt}` is the question and `{system}` the rules the
    /// answer must follow; both appear once. List only tools that cannot
    /// change anything: xherd relies on it.
    pub argv: Vec<String>,
    /// Appended when the TODO names a model; `{model}` is its name.
    #[serde(default)]
    pub model_args: Vec<String>,
}

impl Check {
    /// The argv for one check; the model only when one was chosen.
    pub fn command(&self, prompt: &str, system: &str, model: Option<&str>) -> Vec<String> {
        let sub = |a: &String| {
            a.replace("{prompt}", prompt)
                .replace("{system}", system)
                .replace("{model}", model.unwrap_or_default())
        };
        let mut out: Vec<String> = self.argv.iter().map(sub).collect();
        if model.is_some() {
            out.extend(self.model_args.iter().map(sub));
        }
        out
    }
}

/// Token usage in a runtime's transcript: JSON lines, each possibly holding an
/// object of counters. The plugin says where the counters are and what they
/// are called; xherd decides what counts (`Totals::budget_tokens`), the same
/// for every runtime.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Usage {
    /// JSON pointer (RFC 6901) from a transcript line to its counters object;
    /// lines without it are skipped.
    pub records: String,
    /// JSON pointer to a message id. A runtime that writes one message as
    /// several lines repeats the id: the last line of an id counts.
    #[serde(default)]
    pub id: Option<String>,
    /// Counter name inside the records object, per standard counter.
    pub fields: UsageFields,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsageFields {
    #[serde(default)]
    pub input: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub cache_write: Option<String>,
    #[serde(default)]
    pub cache_read: Option<String>,
}

/// The standard counters, whatever the runtime calls them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
}

impl Totals {
    /// What a token budget counts: input, output and cache writes. Cache reads
    /// repeat the whole context on every turn and would swamp a budget.
    pub fn budget_tokens(&self) -> u64 {
        self.input + self.output + self.cache_write
    }

    fn add(&mut self, o: Totals) {
        self.input += o.input;
        self.output += o.output;
        self.cache_write += o.cache_write;
        self.cache_read += o.cache_read;
    }
}

impl Usage {
    fn check(&self) -> Result<(), String> {
        for p in [Some(&self.records), self.id.as_ref()]
            .into_iter()
            .flatten()
        {
            if !p.starts_with('/') {
                return Err(format!("usage pointer {p:?} must start with /"));
            }
        }
        let f = &self.fields;
        if [&f.input, &f.output, &f.cache_write, &f.cache_read]
            .iter()
            .all(|x| x.is_none())
        {
            return Err("usage.fields names no counter".into());
        }
        Ok(())
    }

    /// Totals of a whole transcript: JSON lines (lines that are not JSON are
    /// skipped), or one JSON document (an export) read as a single line.
    pub fn totals(&self, transcript: &str) -> Totals {
        // A counter name, or a JSON pointer inside the record (`/cache/write`).
        let get = |o: &Value, name: &Option<String>| {
            name.as_deref()
                .and_then(|n| match n.starts_with('/') {
                    true => o.pointer(n),
                    false => o.get(n),
                })
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        let mut by_id: BTreeMap<String, Totals> = BTreeMap::new();
        let mut sum = Totals::default();
        let docs: Vec<Value> = match serde_json::from_str::<Value>(transcript) {
            Ok(v) => vec![v],
            Err(_) => transcript
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect(),
        };
        for v in docs {
            let Some(o) = v.pointer(&self.records) else {
                continue;
            };
            let t = Totals {
                input: get(o, &self.fields.input),
                output: get(o, &self.fields.output),
                cache_write: get(o, &self.fields.cache_write),
                cache_read: get(o, &self.fields.cache_read),
            };
            match self
                .id
                .as_deref()
                .and_then(|p| v.pointer(p))
                .and_then(Value::as_str)
            {
                Some(id) => {
                    by_id.insert(id.to_string(), t);
                }
                None => sum.add(t),
            }
        }
        for t in by_id.into_values() {
            sum.add(t);
        }
        sum
    }
}

/// Continue a runtime conversation by its session id, when the runtime still
/// has its transcript. The transcript is a file (`transcripts` + `file`), or
/// what a command prints for runtimes that keep sessions in a database
/// (`export`, opencode).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resume {
    /// Extra launch arguments; `{session}` is the session id.
    pub args: Vec<String>,
    /// Folder holding one subfolder per project; may use `${VAR}` and `~/`.
    #[serde(default)]
    pub transcripts: Option<String>,
    /// Transcript file name inside a project folder; `{session}` as above,
    /// and at most one `*` for the rest of a name (Codex's
    /// `rollout-*-{session}.jsonl`).
    #[serde(default)]
    pub file: Option<String>,
    /// How many folders down from `transcripts` the file is: 1 (default,
    /// `<project>/<file>`) to 4 (Codex: `<year>/<month>/<day>/<file>`).
    #[serde(default = "one")]
    pub depth: u8,
    /// Launch arguments, after `args`, that give a continued session its
    /// first prompt (`{prompt}` once): for a runtime that takes one there
    /// (`codex resume <id> <prompt>`) and starts no session before it.
    #[serde(default)]
    pub prompt_args: Vec<String>,
    /// A command writing the session's transcript, `{session}` once; it
    /// fails when the runtime no longer has the session. It prints it on
    /// stdout, or writes it to `{file}` (a path xherd gives) when a pipe is
    /// not reliable (opencode drops output past 64 KiB through a pipe).
    #[serde(default)]
    pub export: Vec<String>,
    /// Variables that give a continued session its first prompt, with
    /// `{prompt}` and `{session}`: for a runtime that ignores `promptArgs`
    /// when continuing (opencode's plugin submits it from them).
    #[serde(default)]
    pub prompt_env: BTreeMap<String, String>,
}

fn one() -> u8 {
    1
}

/// Session ids come from the runtime's hook payload; only plain ids are used.
pub fn plain_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl Resume {
    /// The launch arguments for `session`; `None` for an odd id.
    pub fn argv(&self, session: &str) -> Option<Vec<String>> {
        plain_id(session).then(|| {
            self.args
                .iter()
                .map(|a| a.replace("{session}", session))
                .collect()
        })
    }

    /// The export command for `session`, writing to `file` when it takes
    /// one; `None` without a command or for an odd id.
    pub fn export_argv(&self, session: &str, file: &str) -> Option<Vec<String>> {
        (plain_id(session) && !self.export.is_empty()).then(|| {
            self.export
                .iter()
                .map(|a| a.replace("{session}", session).replace("{file}", file))
                .collect()
        })
    }

    /// `env K=V…` giving `session` its first `prompt`; `None` without
    /// `promptEnv`.
    pub fn prompt_env_argv(&self, session: &str, prompt: &str) -> Option<Vec<String>> {
        (!self.prompt_env.is_empty()).then(|| {
            std::iter::once("env".to_string())
                .chain(self.prompt_env.iter().map(|(k, v)| {
                    format!(
                        "{k}={}",
                        v.replace("{session}", session).replace("{prompt}", prompt)
                    )
                }))
                .collect()
        })
    }

    /// `promptArgs` with `prompt`; `None` without them.
    pub fn prompt_argv(&self, prompt: &str) -> Option<Vec<String>> {
        Terminal::prompt_argv(&self.prompt_args, prompt)
    }

    /// The export writes to `{file}` instead of stdout.
    pub fn exports_to_file(&self) -> bool {
        self.export.iter().any(|a| a.contains("{file}"))
    }

    fn check(&self) -> Result<(), String> {
        if !self.prompt_env.is_empty() && !self.prompt_env.values().any(|v| v.contains("{prompt}"))
        {
            return Err("resume.promptEnv needs {prompt}".into());
        }
        if !self.prompt_args.is_empty() {
            if !self.prompt_env.is_empty() {
                return Err("resume takes promptEnv or promptArgs, not both".into());
            }
            if self
                .prompt_args
                .iter()
                .filter(|a| a.contains("{prompt}"))
                .count()
                != 1
            {
                return Err("resume.promptArgs needs {prompt} once".into());
            }
        }
        if !(1..=4).contains(&self.depth) {
            return Err("resume.depth must be 1 to 4".into());
        }
        if self
            .file
            .as_deref()
            .is_some_and(|f| f.matches('*').count() > 1 || f.contains('/'))
        {
            return Err("resume.file is a file name with at most one *".into());
        }
        let n = |p: &str| self.export.iter().filter(|a| a.contains(p)).count();
        let placed = n("{session}") == 1 && n("{file}") <= 1;
        match (&self.transcripts, &self.file, self.export.is_empty()) {
            (Some(_), Some(_), true) => Ok(()),
            (None, None, false) if placed => Ok(()),
            (None, None, false) => {
                Err("resume.export needs {session} once, {file} at most once".into())
            }
            _ => Err("resume needs either transcripts and file, or export".into()),
        }
    }
}

/// Where the runtime records the folders it trusts: a JSON or TOML file whose
/// `projects` map has absolute folder paths as keys and `key: value` once a
/// folder is trusted. xherd sets that key for each agent's own worktree, and
/// only there (design §4.1), so no agent stops at the runtime's trust dialog.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Trust {
    /// May use `${VAR}`, `${VAR:-default}` and a leading `~/`.
    pub file: String,
    pub projects: String,
    pub key: String,
    /// What `key` is set to: `true` (Claude Code), or a string (Codex's
    /// `trust_level = "trusted"`).
    #[serde(default = "trust_true")]
    pub value: Value,
    #[serde(default)]
    pub format: TrustFormat,
}

fn trust_true() -> Value {
    Value::Bool(true)
}

/// The trust file's syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrustFormat {
    #[default]
    Json,
    /// Edited in place: the user's comments and layout are kept (Codex's
    /// `config.toml`).
    Toml,
}

impl Trust {
    /// The trust file's new text with `dir` (as given and canonicalised,
    /// `/tmp` vs `/private/tmp`) trusted; every other key kept. `None` when
    /// nothing changes. Text that does not parse is refused, never replaced.
    pub fn with_trusted(&self, text: Option<&str>, dir: &Path) -> Result<Option<String>, String> {
        let mut forms = vec![dir.to_path_buf()];
        forms.extend(dir.canonicalize().ok().filter(|c| c != dir));
        let forms: Vec<String> = forms
            .iter()
            .map(|f| f.to_string_lossy().into_owned())
            .collect();
        match self.format {
            TrustFormat::Json => self.json_trusted(text, &forms),
            TrustFormat::Toml => self.toml_trusted(text, &forms),
        }
    }

    fn toml_trusted(&self, text: Option<&str>, forms: &[String]) -> Result<Option<String>, String> {
        use toml_edit::{DocumentMut, Item, Table, value};
        let mut doc: DocumentMut = text
            .unwrap_or("")
            .parse()
            .map_err(|_| "not valid TOML; left untouched".to_string())?;
        let want = match &self.value {
            Value::Bool(b) => value(*b),
            Value::String(s) => value(s.as_str()),
            _ => return Err("trust value must be true or a string".into()),
        };
        let projects = doc.entry(&self.projects).or_insert_with(|| {
            let mut t = Table::new();
            t.set_implicit(true);
            Item::Table(t)
        });
        let Some(projects) = projects.as_table_like_mut() else {
            return Err(format!(
                "{:?} is not a table; left untouched",
                self.projects
            ));
        };
        let mut changed = false;
        for f in forms {
            let entry = projects
                .entry(f)
                .or_insert_with(|| Item::Table(Table::new()));
            let Some(e) = entry.as_table_like_mut() else {
                return Err(format!("{f} is not a table; left untouched"));
            };
            let same = e
                .get(&self.key)
                .and_then(Item::as_value)
                .is_some_and(|v| v.to_string().trim() == want.to_string().trim());
            if !same {
                e.insert(&self.key, want.clone());
                changed = true;
            }
        }
        Ok(changed.then(|| doc.to_string()))
    }

    fn json_trusted(&self, text: Option<&str>, forms: &[String]) -> Result<Option<String>, String> {
        let mut root: Map<String, Value> = match text.map(str::trim) {
            None | Some("") => Map::new(),
            Some(t) => match serde_json::from_str(t) {
                Ok(Value::Object(m)) => m,
                _ => return Err("not a JSON object; left untouched".into()),
            },
        };
        let projects = root
            .entry(self.projects.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        let Some(projects) = projects.as_object_mut() else {
            return Err(format!(
                "{:?} is not an object; left untouched",
                self.projects
            ));
        };
        let mut changed = false;
        for f in forms {
            let entry = projects
                .entry(f.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            let Some(e) = entry.as_object_mut() else {
                return Err(format!("{f} is not an object; left untouched"));
            };
            if e.get(&self.key) != Some(&self.value) {
                e.insert(self.key.clone(), self.value.clone());
                changed = true;
            }
        }
        Ok(changed.then(|| serde_json::to_string_pretty(&root).expect("json")))
    }
}

/// The file in the worktree that makes the runtime call `xherd hook <event>`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HookFile {
    /// Worktree-relative path (`.claude/settings.local.json`).
    pub file: String,
    /// Runtime event name → `xherd hook` event.
    pub events: BTreeMap<String, String>,
    #[serde(default)]
    pub format: HookFormat,
}

/// How the hook file is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HookFormat {
    /// A JSON settings file whose `hooks` map runs a command per event; the
    /// user's entries are kept (Claude Code).
    #[default]
    ClaudeSettings,
    /// A JavaScript plugin file owned by xherd (opencode loads every file in
    /// `.opencode/plugin/`); events are the triggers in `OPENCODE_TRIGGERS`.
    OpencodePlugin,
    /// Codex's `.codex/hooks.json`: the same `hooks` map as Claude Code's,
    /// the user's entries kept, but no `permissions` (Codex has its own
    /// sandbox and approval policy, set by the permission mode).
    CodexHooks,
}

/// What the opencode plugin can report: its own start, then opencode hooks
/// and bus events.
pub const OPENCODE_TRIGGERS: &[&str] = &[
    "start",
    "chat.message",
    "tool.execute.after",
    "permission.asked",
    "question.asked",
    "session.idle",
];

/// What xherd's commands need without asking: shell commands (patterns with
/// a trailing `*`) and folders outside the worktree they read (R-005b).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Allow {
    pub commands: Vec<String>,
    pub read_dirs: Vec<String>,
}

impl Terminal {
    pub fn of(m: &Manifest) -> Result<Terminal, String> {
        let t: Terminal = m
            .data
            .get("terminal")
            .cloned()
            .ok_or_else(|| format!("runtime {} has no terminal driver", m.name))
            .and_then(|v| {
                serde_json::from_value(v).map_err(|e| format!("runtime {}: {e}", m.name))
            })?;
        if t.launch.is_empty() {
            return Err(format!("runtime {}: empty launch argv", m.name));
        }
        for (mode, a) in &t.permission_modes {
            if !PERMISSION_MODES.contains(&mode.as_str()) {
                return Err(format!(
                    "runtime {}: unknown permission mode {mode:?} (modes: {})",
                    m.name,
                    PERMISSION_MODES.join(", ")
                ));
            }
            if a.args.is_empty() && a.env.is_empty() {
                return Err(format!(
                    "runtime {}: permission mode {mode} needs explicit flags (the runtime's default may change)",
                    m.name
                ));
            }
            let plain = |k: &String| {
                !k.is_empty()
                    && k.chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            };
            if let Some(k) = a.env.keys().find(|k| !plain(k)) {
                return Err(format!(
                    "runtime {}: permission mode {mode}: variable name {k:?} is not A-Z, 0-9, _",
                    m.name
                ));
            }
        }
        if let Some(h) = &t.hooks
            && h.format == HookFormat::OpencodePlugin
            && let Some(e) = h
                .events
                .keys()
                .find(|e| !OPENCODE_TRIGGERS.contains(&e.as_str()))
        {
            return Err(format!(
                "runtime {}: the opencode plugin cannot report {e:?} (it reports: {})",
                m.name,
                OPENCODE_TRIGGERS.join(", ")
            ));
        }
        if let Some(r) = &t.resume {
            r.check().map_err(|e| format!("runtime {}: {e}", m.name))?;
        }
        if !t.prompt_args.is_empty()
            && t.prompt_args
                .iter()
                .filter(|a| a.contains("{prompt}"))
                .count()
                != 1
        {
            return Err(format!(
                "runtime {}: promptArgs needs {{prompt}} once",
                m.name
            ));
        }
        if let Some(c) = &t.check {
            let n = |p: &str| c.argv.iter().filter(|a| a.contains(p)).count();
            if c.argv.is_empty() || n("{prompt}") != 1 || n("{system}") != 1 {
                return Err(format!(
                    "runtime {}: check needs a command with {{prompt}} and {{system}} once each",
                    m.name
                ));
            }
        }
        if let Some(u) = &t.usage {
            if t.resume.is_none() {
                return Err(format!(
                    "runtime {}: usage needs resume (it names where the transcripts are)",
                    m.name
                ));
            }
            u.check().map_err(|e| format!("runtime {}: {e}", m.name))?;
        }
        for required in [STRICT_PERMISSION_MODE, DEFAULT_PERMISSION_MODE] {
            if !t.permission_modes.contains_key(required) {
                return Err(format!(
                    "runtime {}: permission mode {required} is required",
                    m.name
                ));
            }
        }
        Ok(t)
    }

    /// Launch argv with the arguments of a permission mode; its variables
    /// come first through `env`, so the argv stays one command (resume
    /// arguments are appended to it).
    pub fn argv(&self, mode: &str) -> Result<Vec<String>, String> {
        let m = self
            .permission_modes
            .get(mode)
            .ok_or_else(|| format!("permission mode {mode:?} is not supported"))?;
        let env: Vec<String> = match m.env.is_empty() {
            true => Vec::new(),
            false => std::iter::once("env".to_string())
                .chain(m.env.iter().map(|(k, v)| format!("{k}={v}")))
                .collect(),
        };
        Ok(env
            .into_iter()
            .chain(self.launch.iter().cloned())
            .chain(m.args.iter().cloned())
            .collect())
    }
}

/// The hook file's new content in its format (R-005b).
pub fn hook_file_text(
    existing: Option<&str>,
    h: &HookFile,
    xherd_bin: &str,
    allow: &Allow,
) -> Result<String, String> {
    match h.format {
        HookFormat::ClaudeSettings => merge_hooks(existing, h, xherd_bin, allow),
        HookFormat::OpencodePlugin => opencode_plugin(existing, h, xherd_bin, allow),
        HookFormat::CodexHooks => merge_hooks(existing, h, xherd_bin, &Allow::default()),
    }
}

const OPENCODE_PLUGIN: &str = include_str!("opencode-plugin.js");

/// The opencode plugin file: xherd's own, so it is replaced whole. A file of
/// that name that xherd did not write is refused, never overwritten.
fn opencode_plugin(
    existing: Option<&str>,
    h: &HookFile,
    xherd_bin: &str,
    allow: &Allow,
) -> Result<String, String> {
    const MARK: &str = "// Written by xherd";
    debug_assert!(OPENCODE_PLUGIN.starts_with(MARK));
    if let Some(old) = existing.filter(|t| !t.trim().is_empty())
        && !old.starts_with(MARK)
    {
        return Err(format!("{} exists and was not written by xherd", h.file));
    }
    // JSON is a JavaScript literal.
    let js = |v: Value| v.to_string();
    let dirs: Vec<&str> = allow
        .read_dirs
        .iter()
        .map(|d| d.trim_end_matches('/'))
        .collect();
    Ok(OPENCODE_PLUGIN
        .replace("__XHERD__", &js(json!(xherd_bin)))
        .replace("__EVENTS__", &js(json!(h.events)))
        .replace("__COMMANDS__", &js(json!(allow.commands)))
        .replace("__READ_DIRS__", &js(json!(dirs))))
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Claude Code's form of `allow`: `Read(//dir/**)` (`//` marks an absolute
/// path; one `/` is relative to the project root), then `Bash(command)`.
pub fn claude_rules(allow: &Allow) -> Vec<String> {
    allow
        .read_dirs
        .iter()
        .map(|d| format!("Read(//{}/**)", d.trim_matches('/')))
        .chain(allow.commands.iter().map(|c| format!("Bash({c})")))
        .collect()
}

/// The hook file's new content: `existing` with xherd's tagged entries
/// replaced and everything else kept. Invalid existing JSON is refused (R-005b).
/// `allow` rules (what xherd's own commands need: reading the folder it
/// installed, calling `xherd`) are added to `permissions.allow` unless already
/// there, so the agent does not stop to ask for them.
pub fn merge_hooks(
    existing: Option<&str>,
    h: &HookFile,
    xherd_bin: &str,
    allow: &Allow,
) -> Result<String, String> {
    let allow_rules = claude_rules(allow);
    let mut root: Map<String, Value> = match existing.map(str::trim) {
        None | Some("") => Map::new(),
        Some(text) => match serde_json::from_str(text) {
            Ok(Value::Object(m)) => m,
            Ok(_) => return Err(format!("{} is not a JSON object", h.file)),
            Err(e) => return Err(format!("{} is not valid JSON: {e}", h.file)),
        },
    };
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(hooks) = hooks else {
        return Err(format!("{}: \"hooks\" is not an object", h.file));
    };
    let ours = |v: &Value| {
        v["command"]
            .as_str()
            .is_some_and(|c| c.trim_end().ends_with(HOOK_TAG))
    };
    // Drop our previous entries everywhere (events may have been renamed).
    for groups in hooks.values_mut() {
        if let Value::Array(groups) = groups {
            for g in groups.iter_mut() {
                if let Some(Value::Array(list)) = g.get_mut("hooks") {
                    list.retain(|v| !ours(v));
                }
            }
            groups.retain(|g| g["hooks"].as_array().is_none_or(|l| !l.is_empty()));
        }
    }
    hooks.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
    for (event, ours) in &h.events {
        let command = format!("{} hook {ours} {HOOK_TAG}", shell_quote(xherd_bin));
        let entry = json!({"hooks": [{"type": "command", "command": command}]});
        match hooks
            .entry(event.clone())
            .or_insert_with(|| Value::Array(Vec::new()))
        {
            Value::Array(groups) => groups.push(entry),
            _ => return Err(format!("{}: hooks.{event} is not a list", h.file)),
        }
    }
    if !allow_rules.is_empty() {
        let perms = root
            .entry("permissions")
            .or_insert_with(|| Value::Object(Map::new()));
        let allow = perms
            .as_object_mut()
            .ok_or_else(|| format!("{}: \"permissions\" is not an object", h.file))?
            .entry("allow")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| format!("{}: permissions.allow is not a list", h.file))?;
        for rule in allow_rules {
            let rule = Value::String(rule);
            if !allow.contains(&rule) {
                allow.push(rule);
            }
        }
    }
    let mut out = serde_json::to_string_pretty(&Value::Object(root)).expect("json");
    out.push('\n');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude() -> Terminal {
        let m = crate::builtins()
            .into_iter()
            .find(|m| m.name == "claude-code")
            .unwrap();
        Terminal::of(&m).unwrap()
    }

    #[test]
    fn permission_modes_are_explicit_and_standard() {
        let mut m = crate::builtins()
            .into_iter()
            .find(|m| m.name == "claude-code")
            .unwrap();
        let set =
            |m: &mut Manifest, v: Value| m.data.get_mut("terminal").unwrap()["permissionModes"] = v;
        set(&mut m, json!({"ask": []}));
        assert!(Terminal::of(&m).unwrap_err().contains("explicit flags"));
        set(&mut m, json!({"ask": ["-x"], "yolo": ["-y"]}));
        assert!(
            Terminal::of(&m)
                .unwrap_err()
                .contains("unknown permission mode")
        );
        set(&mut m, json!({"edits": ["-e"]}));
        assert!(Terminal::of(&m).unwrap_err().contains("ask is required"));
        set(&mut m, json!({"ask": ["-a"]}));
        assert!(Terminal::of(&m).unwrap_err().contains("edits is required"));
        set(&mut m, json!({"ask": ["-a"], "edits": ["-e"]}));
        assert!(Terminal::of(&m).is_ok());
    }

    #[test]
    fn builtin_claude_code() {
        let t = claude();
        let mode = |m: &str| t.argv(m).unwrap()[1..].join(" ");
        assert_eq!(mode(STRICT_PERMISSION_MODE), "--permission-mode manual");
        assert_eq!(
            mode(DEFAULT_PERMISSION_MODE),
            "--permission-mode acceptEdits"
        );
        assert_eq!(mode("edits"), "--permission-mode acceptEdits");
        assert_eq!(mode("auto"), "--permission-mode auto");
        assert_eq!(mode("all"), "--permission-mode bypassPermissions");
        let mut keys: Vec<&str> = t.permission_modes.keys().map(String::as_str).collect();
        keys.sort();
        let mut all = PERMISSION_MODES.to_vec();
        all.sort();
        assert_eq!(keys, all);
        assert!(t.argv("nope").is_err());
        let h = t.hooks.unwrap();
        assert_eq!(h.file, ".claude/settings.local.json");
        assert_eq!(h.events["Stop"], "stop");
    }

    #[test]
    fn trust_is_set_for_one_folder_and_keeps_the_rest() {
        let t = claude().trust.unwrap();
        assert_eq!(t.file, "${CLAUDE_CONFIG_DIR:-~}/.claude.json");
        let tmp = tempfile::tempdir().unwrap();
        let wt = tmp.path().join("worktrees/repo/SC-1");
        std::fs::create_dir_all(&wt).unwrap();
        let old = json!({"userID": "u", "projects": {"/other": {"hasTrustDialogAccepted": true, "allowedTools": []}}});
        let new = t
            .with_trusted(Some(&old.to_string()), &wt)
            .unwrap()
            .unwrap();
        let v: Value = serde_json::from_str(&new).unwrap();
        assert_eq!(v["userID"], "u");
        assert_eq!(v["projects"]["/other"]["allowedTools"], json!([]));
        assert_eq!(
            v["projects"][wt.to_string_lossy().as_ref()]["hasTrustDialogAccepted"],
            true
        );
        let canon = wt.canonicalize().unwrap();
        assert_eq!(
            v["projects"][canon.to_string_lossy().as_ref()]["hasTrustDialogAccepted"],
            true
        );
        // Already trusted: nothing to write.
        assert_eq!(t.with_trusted(Some(&new), &wt).unwrap(), None);
        assert!(t.with_trusted(None, &wt).unwrap().is_some());
        assert!(t.with_trusted(Some("{oops"), &wt).is_err());
        assert!(t.with_trusted(Some(r#"{"projects": 3}"#), &wt).is_err());
    }

    #[test]
    fn hooks_merge_keeps_user_entries_and_replaces_ours() {
        let h = claude().hooks.unwrap();
        let user = r#"{"permissions": {"allow": ["Bash(ls)"]},
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}}"#;
        let once = merge_hooks(Some(user), &h, "/opt/x herd/xherd", &Allow::default()).unwrap();
        let twice = merge_hooks(Some(&once), &h, "/usr/bin/xherd", &Allow::default()).unwrap();
        let v: Value = serde_json::from_str(&twice).unwrap();
        assert_eq!(v["permissions"]["allow"][0], "Bash(ls)");
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "user entry kept, ours replaced: {stop:?}");
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
        assert_eq!(
            stop[1]["hooks"][0]["command"],
            "'/usr/bin/xherd' hook stop # xherd"
        );
        assert_eq!(v["hooks"].as_object().unwrap().len(), 5);
        assert!(once.contains(r#"'/opt/x herd/xherd' hook session-start"#));
    }

    #[test]
    fn hooks_merge_refuses_bad_json() {
        let h = claude().hooks.unwrap();
        assert!(merge_hooks(Some("{oops"), &h, "xherd", &Allow::default()).is_err());
        assert!(merge_hooks(Some("[]"), &h, "xherd", &Allow::default()).is_err());
        assert!(merge_hooks(Some(r#"{"hooks": 1}"#), &h, "xherd", &Allow::default()).is_err());
        assert!(
            merge_hooks(None, &h, "xherd", &Allow::default())
                .unwrap()
                .contains("Notification")
        );
    }

    #[test]
    fn read_dirs_are_allowed_once() {
        let h = claude().hooks.unwrap();
        let user = r#"{"permissions": {"allow": ["Bash(ls)"]}}"#;
        let rules = Allow {
            commands: vec!["xherd task *".into()],
            read_dirs: vec!["/home/x/.claude/commands/xherd".into()],
        };
        let once = merge_hooks(Some(user), &h, "xherd", &rules).unwrap();
        let twice = merge_hooks(Some(&once), &h, "xherd", &rules).unwrap();
        let v: Value = serde_json::from_str(&twice).unwrap();
        assert_eq!(
            v["permissions"]["allow"],
            json!([
                "Bash(ls)",
                "Read(//home/x/.claude/commands/xherd/**)",
                "Bash(xherd task *)"
            ])
        );
        let none = merge_hooks(None, &h, "xherd", &Allow::default()).unwrap();
        assert!(!none.contains("permissions"));
    }

    #[test]
    fn claude_usage_counts_each_message_once_and_skips_cache_reads() {
        let u = claude().usage.unwrap();
        let t = r#"{"type":"user","message":{"content":"hi"}}
{"message":{"id":"m1","usage":{"input_tokens":2,"output_tokens":5,"cache_creation_input_tokens":10,"cache_read_input_tokens":9000}}}
{"message":{"id":"m1","usage":{"input_tokens":2,"output_tokens":40,"cache_creation_input_tokens":10,"cache_read_input_tokens":9000}}}
not json
{"message":{"id":"m2","usage":{"input_tokens":1,"output_tokens":1}}}
{"message":{"usage":{"output_tokens":7}}}"#;
        let totals = u.totals(t);
        assert_eq!(
            totals,
            Totals {
                input: 2 + 1,
                output: 40 + 1 + 7,
                cache_write: 10,
                cache_read: 9000
            }
        );
        assert_eq!(totals.budget_tokens(), 3 + 48 + 10);
        assert_eq!(u.totals(""), Totals::default());
    }

    #[test]
    fn another_runtime_declares_its_own_names() {
        let u: Usage = serde_json::from_value(json!({
            "records": "/event/tokens",
            "fields": {"input": "prompt", "output": "completion"}
        }))
        .unwrap();
        u.check().unwrap();
        let t = "{\"event\":{\"tokens\":{\"prompt\":3,\"completion\":4}}}\n{\"event\":{\"tokens\":{\"prompt\":1,\"completion\":1}}}";
        assert_eq!(u.totals(t).budget_tokens(), 9, "no id: every line counts");
    }

    #[test]
    fn usage_declarations_are_checked() {
        let m = |usage: Value, resume: bool| {
            let mut t = json!({
                "launch": ["x"],
                "permissionModes": {"ask": ["a"], "edits": ["e"]},
                "usage": usage
            });
            if resume {
                t["resume"] = json!({"args": ["--r"], "transcripts": "/t", "file": "f"});
            }
            Manifest::parse(
                &json!({"name": "rt", "kind": "runtime", "version": "1", "data": {"terminal": t}})
                    .to_string(),
            )
            .unwrap()
        };
        let ok = json!({"records": "/u", "fields": {"output": "o"}});
        assert!(Terminal::of(&m(ok.clone(), true)).is_ok());
        let e = Terminal::of(&m(ok, false)).unwrap_err();
        assert!(e.contains("needs resume"), "{e}");
        let e =
            Terminal::of(&m(json!({"records": "u", "fields": {"output": "o"}}), true)).unwrap_err();
        assert!(e.contains("must start with /"), "{e}");
        let e = Terminal::of(&m(json!({"records": "/u", "fields": {}}), true)).unwrap_err();
        assert!(e.contains("names no counter"), "{e}");
    }
}

#[cfg(test)]
mod opencode_tests {
    use super::*;

    fn opencode() -> Terminal {
        let m = crate::builtins()
            .into_iter()
            .find(|m| m.name == "opencode")
            .unwrap();
        Terminal::of(&m).unwrap()
    }

    fn permission(argv: &[String]) -> Value {
        assert_eq!(argv[0], "env");
        let v = argv[1].strip_prefix("OPENCODE_PERMISSION=").unwrap();
        serde_json::from_str(v).unwrap()
    }

    #[test]
    fn modes_go_through_the_environment() {
        let t = opencode();
        let mut keys: Vec<&str> = t.permission_modes.keys().map(String::as_str).collect();
        keys.sort();
        let mut all = PERMISSION_MODES.to_vec();
        all.sort();
        assert_eq!(keys, all);
        let ask = t.argv("ask").unwrap();
        assert_eq!(ask[2..], ["opencode"]);
        let p = permission(&ask);
        assert_eq!(
            (p["edit"].as_str(), p["bash"].as_str()),
            (Some("ask"), Some("ask"))
        );
        assert_eq!(p["read"], "allow");
        assert_eq!(permission(&t.argv("edits").unwrap())["edit"], "allow");
        assert_eq!(permission(&t.argv("edits").unwrap())["bash"], "ask");
        let auto = permission(&t.argv("auto").unwrap());
        assert_eq!(
            (auto["bash"].as_str(), auto["external_directory"].as_str()),
            (Some("allow"), Some("ask"))
        );
        let all = t.argv("all").unwrap();
        assert_eq!(all[2..], ["opencode", "--auto"]);
        assert_eq!(permission(&all)["external_directory"], "allow");
    }

    #[test]
    fn the_plugin_file_is_ours_and_filled_in() {
        let h = opencode().hooks.unwrap();
        assert_eq!(h.format, HookFormat::OpencodePlugin);
        let allow = Allow {
            commands: vec!["xherd task *".into()],
            read_dirs: vec!["/home/u/.config/opencode/command/xherd/".into()],
        };
        let js = hook_file_text(None, &h, "/opt/x \"herd\"/xherd", &allow).unwrap();
        assert!(
            js.contains(r#"const XHERD = "/opt/x \"herd\"/xherd";"#),
            "{js}"
        );
        assert!(js.contains(r#""session.idle":"stop""#));
        assert!(js.contains(r#"const COMMANDS = ["xherd task *"];"#));
        assert!(js.contains(r#"const READ_DIRS = ["/home/u/.config/opencode/command/xherd"];"#));
        assert!(!js.contains("__"), "every placeholder filled");
        // Ours is replaced; someone else's is refused.
        assert_eq!(
            hook_file_text(Some(&js), &h, "/opt/x \"herd\"/xherd", &allow).unwrap(),
            js
        );
        let e = hook_file_text(Some("export const Mine = 1"), &h, "xherd", &allow).unwrap_err();
        assert!(e.contains("not written by xherd"), "{e}");
    }

    #[test]
    fn plugin_events_are_checked() {
        let mut m = crate::builtins()
            .into_iter()
            .find(|m| m.name == "opencode")
            .unwrap();
        m.data.get_mut("terminal").unwrap()["hooks"]["events"]["Stop"] = json!("stop");
        assert!(
            Terminal::of(&m)
                .unwrap_err()
                .contains("cannot report \"Stop\"")
        );
    }

    #[test]
    fn resume_and_usage_go_through_the_export() {
        let t = opencode();
        let r = t.resume.unwrap();
        assert_eq!(r.argv("ses_1").unwrap(), ["--session", "ses_1"]);
        assert!(r.exports_to_file());
        let argv = r.export_argv("ses_1", "/w/.xherd/x.json").unwrap();
        assert_eq!(argv[argv.len() - 2..], ["ses_1", "/w/.xherd/x.json"]);
        assert_eq!(r.export_argv("../x", "/f"), None);
        assert_eq!(
            r.prompt_env_argv("ses_1", "/xherd/resume").unwrap(),
            [
                "env",
                "XHERD_PROMPT=/xherd/resume",
                "XHERD_PROMPT_SESSION=ses_1"
            ]
        );
        // `opencode export` prints one pretty JSON document.
        let export = r#"{
  "info": {"id": "ses_1", "tokens": {"input": 100, "output": 20, "reasoning": 5,
           "cache": {"read": 9000, "write": 7}}},
  "messages": []
}"#;
        let totals = t.usage.unwrap().totals(export);
        assert_eq!(
            totals,
            Totals {
                input: 100,
                output: 20,
                cache_write: 7,
                cache_read: 9000
            }
        );
    }

    #[test]
    fn resume_needs_a_file_or_an_export() {
        let r = |v: Value| serde_json::from_value::<Resume>(v).unwrap().check();
        assert!(r(json!({"args": ["-s", "{session}"], "export": ["x", "{session}"]})).is_ok());
        assert!(r(json!({"args": [], "transcripts": "/t", "file": "f"})).is_ok());
        assert!(
            r(json!({"args": [], "export": ["x"]}))
                .unwrap_err()
                .contains("{session} once")
        );
        assert!(r(json!({"args": [], "transcripts": "/t"})).is_err());
        let e = r(json!({"args": [], "export": ["{session}"], "promptEnv": {"X": "{session}"}}));
        assert!(e.unwrap_err().contains("{prompt}"));
        assert!(
            r(json!({"args": [], "transcripts": "/t", "file": "f", "export": ["{session}"]}))
                .is_err()
        );
    }

    #[test]
    fn the_check_is_read_only() {
        let c = opencode().check.unwrap();
        let argv = c.command("PROMPT", "SYSTEM", Some("opencode/m"));
        assert_eq!(argv[2..5], ["opencode", "run", "SYSTEM\n\nPROMPT"]);
        assert_eq!(argv[5..], ["--model", "opencode/m"]);
        let p = permission(&argv);
        assert_eq!(p["*"], "deny");
        assert_eq!(p["edit"], "deny");
        assert_eq!(p["bash"]["*"], "deny");
        assert_eq!(p["bash"]["git log *"], "allow");
    }
}

#[cfg(test)]
mod check_tests {
    use super::*;

    fn check() -> Check {
        Check {
            argv: ["run", "-p", "{prompt}", "--sys", "{system}", "--ro"]
                .map(String::from)
                .to_vec(),
            model_args: ["--model", "{model}"].map(String::from).to_vec(),
        }
    }

    #[test]
    fn the_model_is_added_only_when_chosen() {
        let c = check();
        assert_eq!(
            c.command("is it?", "be careful", None),
            ["run", "-p", "is it?", "--sys", "be careful", "--ro"]
        );
        assert_eq!(c.command("q", "s", Some("m1")).last().unwrap(), "m1");
        assert_eq!(c.command("q", "s", Some("m1")).len(), 8);
    }

    #[test]
    fn the_built_in_claude_code_check_is_valid() {
        let reg = crate::builtins();
        let p = reg.iter().find(|m| m.name == "claude-code").unwrap();
        let t = Terminal::of(p).unwrap();
        let argv = t.check.unwrap().command("PROMPT", "SYSTEM", None);
        assert_eq!(argv[..3], ["claude", "-p", "PROMPT"]);
        assert!(argv.contains(&"--disallowedTools".to_string()));
    }

    fn codex() -> Terminal {
        let m = crate::builtins()
            .into_iter()
            .find(|m| m.name == "codex")
            .unwrap();
        Terminal::of(&m).unwrap()
    }

    #[test]
    fn builtin_codex() {
        let t = codex();
        let mode = |m: &str| t.argv(m).unwrap()[2..].join(" ");
        assert_eq!(t.launch, ["codex", "--dangerously-bypass-hook-trust"]);
        assert_eq!(
            mode("ask"),
            "--sandbox read-only --ask-for-approval on-request"
        );
        assert!(
            mode("edits").starts_with("--sandbox workspace-write --ask-for-approval on-request")
        );
        assert!(mode("edits").ends_with("sandbox_workspace_write.network_access=true"));
        assert!(mode("auto").starts_with("--approve-for-me"));
        assert_eq!(mode("all"), "--dangerously-bypass-approvals-and-sandbox");
        assert_eq!(
            Terminal::prompt_argv(&t.prompt_args, "$xherd-feature SC-1 x").unwrap(),
            ["$xherd-feature SC-1 x"]
        );
        let r = t.resume.as_ref().unwrap();
        assert_eq!(r.argv("s-1").unwrap(), ["resume", "s-1"]);
        assert_eq!(r.prompt_argv("$xherd-resume").unwrap(), ["$xherd-resume"]);
        assert_eq!(
            (r.file.as_deref(), r.depth),
            (Some("rollout-*-{session}.jsonl"), 3)
        );
        let h = t.hooks.as_ref().unwrap();
        assert_eq!(h.format, HookFormat::CodexHooks);
        let allow = Allow {
            commands: vec!["xherd stage enter *".into()],
            read_dirs: vec!["/x".into()],
        };
        let text = hook_file_text(
            Some(r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "mine"}]}]}}"#),
            h,
            "/bin/xherd",
            &allow,
        )
        .unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert!(
            v.get("permissions").is_none(),
            "Codex has no permissions block"
        );
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "mine");
        assert_eq!(
            v["hooks"]["Stop"][1]["hooks"][0]["command"],
            "'/bin/xherd' hook stop # xherd"
        );
        assert_eq!(
            v["hooks"]["PermissionRequest"][0]["hooks"][0]["command"],
            "'/bin/xherd' hook notification # xherd"
        );
        let argv = t.check.as_ref().unwrap().command("P", "S", Some("gpt-x"));
        assert_eq!(argv[..4], ["codex", "exec", "--sandbox", "read-only"]);
        assert!(argv.contains(&"S\n\nP".to_string()));
        assert_eq!(argv[argv.len() - 2..], ["--model", "gpt-x"]);
    }

    #[test]
    fn codex_usage_from_a_rollout() {
        let u = codex().usage.unwrap();
        let line = |id: &str, input: u64| {
            json!({"type": "token_usage_record", "payload": {"response_id": id,
                "usage": {"input_tokens": input, "cached_input_tokens": 10,
                "cache_write_input_tokens": 2, "output_tokens": 6}}})
            .to_string()
        };
        let other = json!({"type": "event_msg", "payload": {"type": "token_count",
            "info": {"total_token_usage": {"input_tokens": 999}}}});
        let text = [line("r1", 100), other.to_string(), line("r2", 50)].join("\n");
        let t = u.totals(&text);
        assert_eq!(
            (t.input, t.output, t.cache_write, t.cache_read),
            (150, 12, 4, 20)
        );
    }

    #[test]
    fn toml_trust_keeps_the_rest() {
        let t = codex().trust.unwrap();
        assert_eq!(t.format, TrustFormat::Toml);
        let dir = Path::new("/no/such/wt/A-1");
        let old = "# mine\nmodel = \"x\"\n\n[projects.\"/elsewhere\"]\ntrust_level = \"trusted\"\n";
        let new = t.with_trusted(Some(old), dir).unwrap().unwrap();
        assert!(new.starts_with("# mine\nmodel = \"x\"\n"), "{new}");
        assert!(
            new.contains("[projects.\"/no/such/wt/A-1\"]\ntrust_level = \"trusted\"\n"),
            "{new}"
        );
        let doc: toml_edit::DocumentMut = new.parse().unwrap();
        assert_eq!(
            doc["projects"]["/elsewhere"]["trust_level"].as_str(),
            Some("trusted")
        );
        assert_eq!(t.with_trusted(Some(&new), dir).unwrap(), None, "unchanged");
        let first = t.with_trusted(None, dir).unwrap().unwrap();
        assert!(first.contains("trust_level = \"trusted\""));
        // Another level is replaced; broken TOML or a non-table is left alone.
        let other = new.replace(
            "[projects.\"/no/such/wt/A-1\"]\ntrust_level = \"trusted\"",
            "[projects.\"/no/such/wt/A-1\"]\ntrust_level = \"untrusted\"",
        );
        assert!(
            t.with_trusted(Some(&other), dir)
                .unwrap()
                .unwrap()
                .contains("[projects.\"/no/such/wt/A-1\"]\ntrust_level = \"trusted\"")
        );
        assert!(t.with_trusted(Some("[projects"), dir).is_err());
        assert!(t.with_trusted(Some("projects = 3"), dir).is_err());
    }

    #[test]
    fn resume_prompt_args_and_depth_are_checked() {
        let r = |v: Value| -> Result<(), String> {
            serde_json::from_value::<Resume>(v).unwrap().check()
        };
        let base = json!({"args": ["resume", "{session}"], "transcripts": "/t", "file": "{session}.jsonl"});
        let with = |k: &str, v: Value| {
            let mut b = base.clone();
            b[k] = v;
            b
        };
        assert!(r(base.clone()).is_ok());
        assert!(r(with("depth", json!(0))).is_err());
        assert!(r(with("depth", json!(5))).is_err());
        assert!(r(with("file", json!("a*b*{session}"))).is_err());
        assert!(r(with("file", json!("x/{session}"))).is_err());
        assert!(r(with("promptArgs", json!(["--p"]))).is_err());
        let mut both = with("promptArgs", json!(["{prompt}"]));
        assert!(r(both.clone()).is_ok());
        both["promptEnv"] = json!({"P": "{prompt}"});
        assert!(r(both).is_err());
    }
}
