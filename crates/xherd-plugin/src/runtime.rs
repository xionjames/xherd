//! Runtime plugins, terminal driver part (R-511): the launch argv, permission
//! modes, and the hook file that makes the runtime call `xherd hook <event>`.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::Manifest;

/// Tool-permission mode used until agents can choose one (spec §13 T1: ask by default).
pub const DEFAULT_PERMISSION_MODE: &str = "ask";

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
pub fn merge_hooks(
    existing: Option<&str>,
    h: &HookFile,
    xherd_bin: &str,
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
    fn builtin_claude_code() {
        let t = claude();
        assert_eq!(t.argv(DEFAULT_PERMISSION_MODE).unwrap(), ["claude"]);
        assert_eq!(
            t.argv("edits").unwrap(),
            ["claude", "--permission-mode", "acceptEdits"]
        );
        assert!(t.argv("nope").is_err());
        let h = t.hooks.unwrap();
        assert_eq!(h.file, ".claude/settings.local.json");
        assert_eq!(h.events["Stop"], "stop");
    }

    #[test]
    fn hooks_merge_keeps_user_entries_and_replaces_ours() {
        let h = claude().hooks.unwrap();
        let user = r#"{"permissions": {"allow": ["Bash(ls)"]},
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}}"#;
        let once = merge_hooks(Some(user), &h, "/opt/x herd/xherd").unwrap();
        let twice = merge_hooks(Some(&once), &h, "/usr/bin/xherd").unwrap();
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
        assert!(merge_hooks(Some("{oops"), &h, "xherd").is_err());
        assert!(merge_hooks(Some("[]"), &h, "xherd").is_err());
        assert!(merge_hooks(Some(r#"{"hooks": 1}"#), &h, "xherd").is_err());
        assert!(
            merge_hooks(None, &h, "xherd")
                .unwrap()
                .contains("Notification")
        );
    }
}
