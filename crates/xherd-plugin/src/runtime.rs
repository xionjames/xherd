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

/// Appended to every hook command xherd writes, so its entries can be found
/// and replaced without touching the user's own hooks (R-005b).
pub const HOOK_TAG: &str = "# xherd";

/// `data.terminal` of a runtime plugin.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Terminal {
    pub launch: Vec<String>,
    /// Mode name → extra launch arguments.
    #[serde(default)]
    pub permission_modes: BTreeMap<String, Vec<String>>,
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

    /// Totals of a whole transcript. Lines that are not JSON are skipped.
    pub fn totals(&self, transcript: &str) -> Totals {
        let get = |o: &Value, name: &Option<String>| {
            name.as_deref()
                .and_then(|n| o.get(n))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        let mut by_id: BTreeMap<String, Totals> = BTreeMap::new();
        let mut sum = Totals::default();
        for line in transcript.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
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
/// has its transcript.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Resume {
    /// Extra launch arguments; `{session}` is the session id.
    pub args: Vec<String>,
    /// Folder holding one subfolder per project; may use `${VAR}` and `~/`.
    pub transcripts: String,
    /// Transcript file name inside a project folder; `{session}` as above.
    pub file: String,
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
}

/// Where the runtime records the folders it trusts: a JSON file whose
/// `projects` map has absolute folder paths as keys and `key: true` once a
/// folder is trusted. xherd sets that key for each agent's own worktree, and
/// only there (design §4.1), so no agent stops at the runtime's trust dialog.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Trust {
    /// May use `${VAR}`, `${VAR:-default}` and a leading `~/`.
    pub file: String,
    pub projects: String,
    pub key: String,
}

impl Trust {
    /// The trust file's new text with `dir` (as given and canonicalised,
    /// `/tmp` vs `/private/tmp`) trusted; every other key kept. `None` when
    /// nothing changes. Text that is not a JSON object is refused, never
    /// replaced.
    pub fn with_trusted(&self, text: Option<&str>, dir: &Path) -> Result<Option<String>, String> {
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
        let mut forms = vec![dir.to_path_buf()];
        forms.extend(dir.canonicalize().ok().filter(|c| c != dir));
        let mut changed = false;
        for f in forms {
            let entry = projects
                .entry(f.to_string_lossy().into_owned())
                .or_insert_with(|| Value::Object(Map::new()));
            let Some(e) = entry.as_object_mut() else {
                return Err(format!("{} is not an object; left untouched", f.display()));
            };
            if e.get(&self.key) != Some(&Value::Bool(true)) {
                e.insert(self.key.clone(), Value::Bool(true));
                changed = true;
            }
        }
        Ok(changed.then(|| serde_json::to_string_pretty(&root).expect("json")))
    }
}

/// A settings file in the worktree whose `hooks` map runs a command per event.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HookFile {
    /// Worktree-relative path (`.claude/settings.local.json`).
    pub file: String,
    /// Runtime event name → `xherd hook` event.
    pub events: BTreeMap<String, String>,
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
        for (mode, args) in &t.permission_modes {
            if !PERMISSION_MODES.contains(&mode.as_str()) {
                return Err(format!(
                    "runtime {}: unknown permission mode {mode:?} (modes: {})",
                    m.name,
                    PERMISSION_MODES.join(", ")
                ));
            }
            if args.is_empty() {
                return Err(format!(
                    "runtime {}: permission mode {mode} needs explicit flags (the runtime's default may change)",
                    m.name
                ));
            }
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

    /// Launch argv with the arguments of a permission mode.
    pub fn argv(&self, mode: &str) -> Result<Vec<String>, String> {
        let extra = self
            .permission_modes
            .get(mode)
            .ok_or_else(|| format!("permission mode {mode:?} is not supported"))?;
        Ok(self.launch.iter().chain(extra).cloned().collect())
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
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
    allow_rules: &[String],
) -> Result<String, String> {
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
            let rule = Value::String(rule.clone());
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
        let once = merge_hooks(Some(user), &h, "/opt/x herd/xherd", &[]).unwrap();
        let twice = merge_hooks(Some(&once), &h, "/usr/bin/xherd", &[]).unwrap();
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
        assert!(merge_hooks(Some("{oops"), &h, "xherd", &[]).is_err());
        assert!(merge_hooks(Some("[]"), &h, "xherd", &[]).is_err());
        assert!(merge_hooks(Some(r#"{"hooks": 1}"#), &h, "xherd", &[]).is_err());
        assert!(
            merge_hooks(None, &h, "xherd", &[])
                .unwrap()
                .contains("Notification")
        );
    }

    #[test]
    fn read_dirs_are_allowed_once() {
        let h = claude().hooks.unwrap();
        let user = r#"{"permissions": {"allow": ["Bash(ls)"]}}"#;
        let rules = ["Read(//home/x/.claude/commands/xherd/**)".to_string()];
        let once = merge_hooks(Some(user), &h, "xherd", &rules).unwrap();
        let twice = merge_hooks(Some(&once), &h, "xherd", &rules).unwrap();
        let v: Value = serde_json::from_str(&twice).unwrap();
        assert_eq!(
            v["permissions"]["allow"],
            json!(["Bash(ls)", "Read(//home/x/.claude/commands/xherd/**)"])
        );
        let none = merge_hooks(None, &h, "xherd", &[]).unwrap();
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
}
