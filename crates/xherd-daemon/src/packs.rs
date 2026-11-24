//! Flow packs loaded at start: the built-in one and the pack of every
//! external flow plugin (R11). A plugin whose pack fails the checks is
//! listed with its error and never stops the others.

use std::collections::BTreeMap;

use xherd_flow::Pack;
use xherd_flow::install::check_plugin_install;
use xherd_plugin::{Kind, Registry};

use crate::Daemon;
use crate::flow_plugin::read_flow_plugin;

/// Where a loaded pack comes from, and the files its commands install from.
#[derive(Debug, Clone)]
pub struct PackSource {
    /// The plugin that ships the pack; `None` for the built-in one.
    pub plugin: Option<String>,
    /// The pack's own files (`FORMAT.md`, the prompts), `plan_install`'s input.
    pub files: Vec<(String, String)>,
}

/// What `load_packs` found: packs by name, their sources, and the plugins
/// whose pack could not be loaded (plugin, error).
pub type Loaded = (
    BTreeMap<String, Pack>,
    BTreeMap<String, PackSource>,
    Vec<(String, String)>,
);

/// The built-in pack, then the pack of every external `flow` plugin, enabled
/// or not. The R12 install-path rule runs with `env`. A failure is a `warn`
/// log and an entry in the third value.
pub fn load_packs(plugins: &Registry, env: &dyn Fn(&str) -> Option<String>) -> Loaded {
    let builtin = xherd_flow::builtin();
    let mut packs = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut errors = Vec::new();
    for p in plugins.of_kind(Kind::Flow) {
        let Some(root) = &p.root else { continue };
        let loaded = read_flow_plugin(root, &p.manifest).and_then(|f| {
            let files: Vec<(&str, &str)> = f
                .files
                .iter()
                .map(|(n, t)| (n.as_str(), t.as_str()))
                .collect();
            check_plugin_install(&f.pack, &builtin, &files, env)?;
            Ok(f)
        });
        match loaded {
            Ok(f) => {
                sources.insert(
                    f.pack.name.clone(),
                    PackSource {
                        plugin: Some(p.name().to_string()),
                        files: f.files,
                    },
                );
                packs.insert(f.pack.name.clone(), f.pack);
            }
            Err(error) => {
                tracing::warn!(plugin = %p.name(), %error, "flow plugin pack not loaded");
                errors.push((p.name().to_string(), error));
            }
        }
    }
    sources.insert(
        xherd_flow::BUILTIN.to_string(),
        PackSource {
            plugin: None,
            files: xherd_flow::install::BUILTIN_FILES
                .iter()
                .map(|(n, t)| (n.to_string(), t.to_string()))
                .collect(),
        },
    );
    packs.insert(xherd_flow::BUILTIN.to_string(), builtin);
    (packs, sources, errors)
}

/// Whether a pack's plugin can serve it now: switched on, and unchanged since
/// start (an install or update waits for a restart, R11, R10).
fn plugin_serves(
    name: &str,
    enabled: bool,
    loaded: &BTreeMap<String, String>,
    state: &BTreeMap<String, String>,
) -> bool {
    enabled && loaded.get(name) == state.get(name)
}

impl Daemon {
    /// A new job may use `pack`: the built-in always, a plugin's pack only
    /// while its plugin is enabled and unchanged since start (R11, R10).
    pub fn pack_selectable(&self, pack: &str) -> bool {
        let Some(source) = self.pack_sources.get(pack) else {
            return false;
        };
        let Some(plugin) = &source.plugin else {
            return true;
        };
        let state = self.plugin_state.lock().unwrap_or_else(|e| e.into_inner());
        plugin_serves(
            plugin,
            self.plugin_enabled(plugin),
            &self.loaded_plugins,
            &state,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_plugin_serves_only_while_enabled_and_unchanged() {
        let map = |sha: &str| BTreeMap::from([("acme-sdd".to_string(), sha.to_string())]);
        assert!(plugin_serves("acme-sdd", true, &map("a"), &map("a")));
        assert!(!plugin_serves("acme-sdd", false, &map("a"), &map("a")));
        assert!(!plugin_serves("acme-sdd", true, &map("a"), &map("b")));
        assert!(!plugin_serves(
            "acme-sdd",
            true,
            &map("a"),
            &BTreeMap::new()
        ));
    }
    use std::path::{Path, PathBuf};

    const PACK: &str = r#"
name: acme-sdd
version: 1.2.0
protocol: 1
specDir: private
memoryNote: required
private: [HANDOFF.md]
install:
  claude-code:
    prefix: "/acme-sdd:"
    commands: { from: commands/*.md, to: "${CLAUDE_CONFIG_DIR:-~/.claude}/commands/acme-sdd/{name}.md", format: command }
    args: "$ARGUMENTS"
commands: { resume: "{prefix}resume", handoff: "{prefix}handoff", status: "{prefix}status", ask: "{prefix}ask {prompt}" }
flows:
  feature:
    start: "{prefix}start {key} {prompt}"
    stages:
      - { id: brief, role: spec, class: critical, gate: { docs: [spec.md] }, next: [{ run: "{prefix}design" }] }
"#;

    const MANIFEST: &str = r#"{"name": "acme-sdd", "kind": "flow", "version": "1.2.0",
        "data": {"flow": {"pack": "pack"}}}"#;

    fn env(k: &str) -> Option<String> {
        (k == "HOME").then(|| "/home/xherd".to_string())
    }

    /// `<tmp>/plugins/acme-sdd`, a flow plugin; returns the plugins folder.
    fn plugins_dir(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("xherd-packs-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let dir = base.join("acme-sdd");
        fs::create_dir_all(dir.join("pack/commands")).unwrap();
        fs::write(dir.join("xherd-plugin.json"), MANIFEST).unwrap();
        fs::write(dir.join("pack/xherd-flow.yaml"), PACK).unwrap();
        for p in [
            "rules", "start", "design", "ask", "resume", "handoff", "status",
        ] {
            fs::write(
                dir.join(format!("pack/commands/{p}.md")),
                format!("# {p}\n"),
            )
            .unwrap();
        }
        base
    }

    fn load(base: &Path) -> Loaded {
        let registry = Registry::load(xherd_plugin::builtins(), base);
        load_packs(&registry, &env)
    }

    #[test]
    fn a_good_plugin_is_loaded_beside_the_builtin() {
        let base = plugins_dir("good");
        let (packs, sources, errors) = load(&base);
        let _ = fs::remove_dir_all(&base);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            packs.keys().map(String::as_str).collect::<Vec<_>>(),
            ["acme-sdd", "xherd"]
        );
        assert_eq!(sources["acme-sdd"].plugin.as_deref(), Some("acme-sdd"));
        assert!(
            sources["acme-sdd"]
                .files
                .iter()
                .any(|(p, _)| p == "commands/design.md")
        );
        assert_eq!(sources["xherd"].plugin, None);
        assert!(!sources["xherd"].files.is_empty());
    }

    #[test]
    fn a_broken_pack_is_an_error_entry_and_the_rest_still_load() {
        let base = plugins_dir("broken");
        fs::write(base.join("acme-sdd/pack/xherd-flow.yaml"), "name: [").unwrap();
        let (packs, sources, errors) = load(&base);
        let _ = fs::remove_dir_all(&base);
        assert_eq!(packs.keys().collect::<Vec<_>>(), ["xherd"]);
        assert_eq!(sources.keys().collect::<Vec<_>>(), ["xherd"]);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "acme-sdd");
    }

    #[test]
    fn a_pack_that_installs_outside_its_folder_is_an_error_entry() {
        let base = plugins_dir("escape");
        let yaml = PACK.replace("/commands/acme-sdd/{name}.md", "/.ssh/{name}.md");
        fs::write(base.join("acme-sdd/pack/xherd-flow.yaml"), yaml).unwrap();
        let (packs, _, errors) = load(&base);
        let _ = fs::remove_dir_all(&base);
        assert!(!packs.contains_key("acme-sdd"));
        assert_eq!(errors.len(), 1, "{errors:?}");
    }
}
