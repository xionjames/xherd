//! Rendering a pack's command files for a runtime (design §6.7). Writing them
//! and keeping user edits is the daemon's job (`xherd install`).

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use crate::Pack;

/// The built-in pack's files, by path inside `packs/xherd/`.
pub const BUILTIN_FILES: &[(&str, &str)] = &[
    ("FORMAT.md", include_str!("../../../packs/xherd/FORMAT.md")),
    (
        "commands/ask.md",
        include_str!("../../../packs/xherd/commands/ask.md"),
    ),
    (
        "commands/bugfix.md",
        include_str!("../../../packs/xherd/commands/bugfix.md"),
    ),
    (
        "commands/commit.md",
        include_str!("../../../packs/xherd/commands/commit.md"),
    ),
    (
        "commands/conflicts.md",
        include_str!("../../../packs/xherd/commands/conflicts.md"),
    ),
    (
        "commands/design.md",
        include_str!("../../../packs/xherd/commands/design.md"),
    ),
    (
        "commands/feature.md",
        include_str!("../../../packs/xherd/commands/feature.md"),
    ),
    (
        "commands/handoff.md",
        include_str!("../../../packs/xherd/commands/handoff.md"),
    ),
    (
        "commands/next.md",
        include_str!("../../../packs/xherd/commands/next.md"),
    ),
    (
        "commands/pr-apply.md",
        include_str!("../../../packs/xherd/commands/pr-apply.md"),
    ),
    (
        "commands/pr-review.md",
        include_str!("../../../packs/xherd/commands/pr-review.md"),
    ),
    (
        "commands/reproduce.md",
        include_str!("../../../packs/xherd/commands/reproduce.md"),
    ),
    (
        "commands/resume.md",
        include_str!("../../../packs/xherd/commands/resume.md"),
    ),
    (
        "commands/review.md",
        include_str!("../../../packs/xherd/commands/review.md"),
    ),
    (
        "commands/rules.md",
        include_str!("../../../packs/xherd/commands/rules.md"),
    ),
    (
        "commands/ship.md",
        include_str!("../../../packs/xherd/commands/ship.md"),
    ),
    (
        "commands/status.md",
        include_str!("../../../packs/xherd/commands/status.md"),
    ),
    (
        "commands/tasks.md",
        include_str!("../../../packs/xherd/commands/tasks.md"),
    ),
];

/// What `{{xap}}` expands to when the runtime calls XAP through the shell.
pub const XAP_SHELL: &str = r#"## Calling xherd (XAP)

Run the calls with the shell, exactly as shown, each one as its own command
(`xherd` is on your `PATH`; do not replace it with a variable, do not chain it
with `;` or `&&`, and do not append `echo $?`: the exit status comes back with
the result, and a command with a variable asks the user every time). Document paths are relative to `.xherd/spec/`.

| Exit | Meaning | Do |
|---|---|---|
| 0 | accepted | continue |
| 2 | refused | read the one-line message on stderr, fix the cause, call again |
| 3 | xherd unreachable | stop and wait; do not retry in a loop |
| 4 | not under xherd | skip the call |

The calls this flow uses:

- `xherd stage enter spec` when a step starts.
- `xherd gate open spec` when its documents are ready; then stop.
- `xherd ask "Which database should the cache use?"` for a question without documents; then stop.
- `xherd tasks sync` after writing or changing `tasks.md`.
- `xherd task start T01`, `xherd task done T01 --evidence "cargo test: 40 passed"`, `xherd task block T01 --reason "Waiting for the API key"`.
- `xherd commit propose C01 --subject "Add the cache" --message "Add the cache: …" --files src/cache.rs,src/lib.rs --tasks T01`; then stop. xherd commits it once the user approves, with that message.
- `xherd commit done C01 --sha 3f2a9c1` to record a commit (xherd's, or yours after git refused it); `xherd commit drop C01 --reason "Folded into C02"` to abandon one.
- `xherd pr opened --url https://github.com/acme/app/pull/42`, `xherd handoff written`.
- `xherd issue add --title "Retry loop ignores the timeout" --severity major` for problems outside the task."#;

/// A skill's `agents/openai.yaml` (Codex): xherd's skills run only when
/// called by name (`$xherd-next`), never because the model picked one.
pub const SKILL_POLICY: &str = "policy:\n  allow_implicit_invocation: false\n";

/// A command file as a skill's `SKILL.md`: frontmatter `name` (the skill's
/// folder) and `description` only, then the body. The command's
/// `description:` line is read as plain text, as Claude Code and opencode
/// do (`feature.md`'s has a `: ` that YAML would refuse).
fn as_skill(text: &str, name: &str) -> Result<String, String> {
    let (front, body) = text
        .strip_prefix("---\n")
        .and_then(|t| t.split_once("\n---\n"))
        .ok_or("a skill needs frontmatter")?;
    let description = front
        .lines()
        .find_map(|l| l.strip_prefix("description:"))
        .map(|d| d.trim().trim_matches('"').to_string())
        .filter(|d| !d.is_empty())
        .ok_or("a skill needs a description in its frontmatter")?;
    Ok(format!(
        "---\nname: {name}\ndescription: {}\n---\n{body}",
        serde_json::Value::String(description)
    ))
}

/// One rendered file and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub path: PathBuf,
    pub text: String,
}

/// Replace `{{name}}` placeholders; an unknown name is an error.
pub fn render_file(text: &str, vars: &[(&str, &str)]) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let end = after.find("}}").ok_or("unclosed '{{'")?;
        let name = after[..end].trim();
        let value = vars
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| *v)
            .ok_or_else(|| format!("unknown placeholder {{{{{name}}}}}"))?;
        out.push_str(value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `${VAR}`, `${VAR:-default}` and a leading `~/` in a path (install
/// destinations, runtime trust files).
pub fn expand(path: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = path;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| format!("unclosed ${{ in {path:?}"))?;
        let (var, default) = match after[..end].split_once(":-") {
            Some((v, d)) => (v, Some(d)),
            None => (&after[..end], None),
        };
        let value = env(var)
            .filter(|v| !v.is_empty())
            .or_else(|| default.map(str::to_string))
            .ok_or_else(|| format!("{var} is not set (needed by {path:?})"))?;
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    if let Some(r) = out.strip_prefix("~/") {
        let home = env("HOME").ok_or("HOME is not set")?;
        out = format!("{}/{r}", home.trim_end_matches('/'));
    }
    Ok(out)
}

/// `commands/*.md` against `commands/next.md` → `Some("next")`.
fn glob_name<'a>(glob: &str, path: &'a str) -> Option<&'a str> {
    let (pre, post) = glob.split_once('*')?;
    let name = path.strip_prefix(pre)?.strip_suffix(post)?;
    (!name.is_empty() && !name.contains('/')).then_some(name)
}

impl Pack {
    /// Every command file of the pack rendered for `runtime`, with its
    /// destination. `files` are the pack's files (path inside the pack, text);
    /// `env` resolves `${VAR}` and `HOME`.
    pub fn plan_install(
        &self,
        runtime: &str,
        files: &[(&str, &str)],
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Vec<Planned>, String> {
        let inst = self
            .install
            .get(runtime)
            .ok_or_else(|| format!("pack {} does not support runtime {runtime}", self.name))?;
        let format = files
            .iter()
            .find(|(p, _)| *p == "FORMAT.md")
            .map_or("", |(_, t)| *t);
        let mut out = Vec::new();
        for set in [&inst.commands, &inst.agents].into_iter().flatten() {
            let skill = match set.format.as_str() {
                "command" => false,
                "skill" => true,
                f => {
                    return Err(format!(
                        "install.{runtime}: format {f:?} is not supported yet"
                    ));
                }
            };
            let dest = |name: &str| -> Result<PathBuf, String> {
                let to = crate::template::render(&set.to, &[("name", name)])?;
                Ok(PathBuf::from(expand(&to, env)?))
            };
            // `{{path:NAME}}`: where another command of the set is installed.
            let mut paths = Vec::new();
            for (path, _) in files {
                if let Some(name) = glob_name(&set.from, path) {
                    let to = dest(name)?.to_string_lossy().into_owned();
                    paths.push((format!("path:{name}"), to));
                }
            }
            for (path, text) in files {
                let Some(name) = glob_name(&set.from, path) else {
                    continue;
                };
                let dest = dest(name)?;
                let dir = dest
                    .parent()
                    .map(|d| d.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let mut vars = vec![
                    ("prefix", inst.prefix.as_str()),
                    ("args", inst.args.as_deref().unwrap_or("")),
                    ("dir", &dir),
                    ("xap", XAP_SHELL),
                    ("format", format),
                ];
                vars.extend(paths.iter().map(|(k, v)| (k.as_str(), v.as_str())));
                let text = render_file(text, &vars).map_err(|e| format!("{path}: {e}"))?;
                if skill {
                    let folder = dest.parent().and_then(|d| d.file_name());
                    let name = folder
                        .map(|f| f.to_string_lossy().into_owned())
                        .ok_or_else(|| format!("{path}: a skill needs its own folder"))?;
                    let text = as_skill(&text, &name).map_err(|e| format!("{path}: {e}"))?;
                    out.push(Planned {
                        path: dest.with_file_name("agents").join("openai.yaml"),
                        text: SKILL_POLICY.into(),
                    });
                    out.push(Planned { path: dest, text });
                } else {
                    out.push(Planned { path: dest, text });
                }
            }
        }
        Ok(out)
    }

    /// The slash commands `runtime` gets from the pack: `{prefix}{name}` for
    /// every file its `commands` set installs, sorted (R12). Unknown runtime:
    /// none.
    pub fn slash_commands(&self, runtime: &str, files: &[(&str, &str)]) -> Vec<String> {
        let Some(inst) = self.install.get(runtime) else {
            return Vec::new();
        };
        let Some(set) = &inst.commands else {
            return Vec::new();
        };
        let mut out: Vec<String> = files
            .iter()
            .filter_map(|(path, _)| glob_name(&set.from, path))
            .map(|name| format!("{}{name}", inst.prefix))
            .collect();
        out.sort();
        out
    }
}

/// The pack's own files in `dir`, as `plan_install` wants them: `FORMAT.md`
/// when present and every file an `install.*.commands`/`agents` `from` glob
/// matches. Anything that is not a regular file is skipped; a matching file
/// that is not UTF-8 is an error.
pub fn read_pack_dir(dir: &Path, pack: &Pack) -> Result<Vec<(String, String)>, String> {
    let mut names: BTreeSet<String> = BTreeSet::new();
    if dir.join("FORMAT.md").is_file() {
        names.insert("FORMAT.md".into());
    }
    let globs = pack
        .install
        .values()
        .flat_map(|i| [&i.commands, &i.agents])
        .flatten()
        .map(|s| s.from.as_str());
    for glob in globs {
        // R12: a `from` glob stays inside the pack folder.
        if Path::new(glob)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(format!(
                "{glob}: `from` must be a relative path inside the pack"
            ));
        }
        let folder = glob.rsplit_once('/').map_or("", |(d, _)| d);
        let Ok(entries) = std::fs::read_dir(dir.join(folder)) else {
            continue;
        };
        for entry in entries {
            let entry = entry.map_err(|e| format!("{glob}: {e}"))?;
            let file = entry.file_name().to_string_lossy().into_owned();
            let path = if folder.is_empty() {
                file
            } else {
                format!("{folder}/{file}")
            };
            if glob_name(glob, &path).is_some() && entry.file_type().is_ok_and(|t| t.is_file()) {
                names.insert(path);
            }
        }
    }
    names
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(dir.join(&p)).map_err(|e| format!("{p}: {e}"))?;
            Ok((p, text))
        })
        .collect()
}

/// The command templates of the pack that no file answers: for each runtime
/// with a `commands` set, `{prefix}<name>` needs the file its `from` glob
/// names for `<name>`. One line per missing template and file.
pub fn missing_prompts(pack: &Pack, files: &[(&str, &str)]) -> Vec<String> {
    let mut templates: Vec<&str> = pack.commands.values().map(String::as_str).collect();
    for f in pack.flows.values() {
        templates.push(&f.start);
        for s in &f.stages {
            templates.extend(s.command.as_deref());
            for r in &s.next {
                templates.push(&r.run);
                templates.extend(r.auto_approved.as_deref());
            }
        }
    }
    let mut missing = Vec::new();
    for t in templates {
        let Some(name) = t
            .strip_prefix("{prefix}")
            .and_then(|r| r.split_whitespace().next())
        else {
            missing.push(format!("{t}: does not start with {{prefix}}"));
            continue;
        };
        for set in pack.install.values().filter_map(|i| i.commands.as_ref()) {
            let file = set.from.replacen('*', name, 1);
            let line = format!("{t}: no {file}");
            if !files.iter().any(|(p, _)| *p == file) && !missing.contains(&line) {
                missing.push(line);
            }
        }
    }
    missing
}

/// The built-in pack's `to` template for `runtime`, split before the first
/// segment that names `xherd` or `{name}`: (folder template, the rest).
/// Cutting the template, not the expanded path, keeps a HOME such as
/// `/home/xherd` from moving the cut.
fn builtin_split<'a>(builtin: &'a Pack, runtime: &str) -> Result<(&'a str, &'a str), String> {
    let inst = builtin
        .install
        .get(runtime)
        .ok_or_else(|| format!("runtime {runtime} is not one of the built-in pack's"))?;
    let set = inst
        .commands
        .as_ref()
        .or(inst.agents.as_ref())
        .ok_or_else(|| format!("install.{runtime}: no commands to install"))?;
    let at = [set.to.find("xherd"), set.to.find("{name}")]
        .into_iter()
        .flatten()
        .min()
        .ok_or_else(|| format!("install.{runtime}: `to` names neither xherd nor {{name}}"))?;
    let cut = set.to[..at]
        .rfind('/')
        .ok_or_else(|| format!("install.{runtime}: `to` has no folder"))?;
    Ok((&set.to[..cut], &set.to[cut + 1..]))
}

/// The folder that holds the built-in pack's files for `runtime`
/// (`~/.claude/commands`, `~/.codex/skills`, `~/.pi/agent/prompts`): the
/// only place a plugin pack may install under (R12).
pub fn runtime_folder(
    builtin: &Pack,
    runtime: &str,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<PathBuf, String> {
    let (dir, _) = builtin_split(builtin, runtime)?;
    Ok(expand(dir, env)?.into())
}

/// R12: where a plugin pack's commands may be installed. For each runtime
/// the pack declares: the built-in declares it too, the `prefix` contains
/// the pack name, and every file `plan_install` plans is a normal path
/// under `runtime_folder` whose first segment has the built-in's shape with
/// the pack name for `xherd` (folder `<pack>`, or `<pack>-…` where the
/// built-in uses `xherd-…`, a flat file for pi).
pub fn check_plugin_install(
    pack: &Pack,
    builtin: &Pack,
    files: &[(&str, &str)],
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<(), String> {
    for (runtime, inst) in &pack.install {
        let (_, rest) = builtin_split(builtin, runtime)?;
        let folder = runtime_folder(builtin, runtime, env)?;
        if !inst.prefix.contains(&pack.name) {
            return Err(format!(
                "install.{runtime}: prefix {:?} does not contain the pack name {}",
                inst.prefix, pack.name
            ));
        }
        // `xherd/{name}.md` → a folder named like the pack; `xherd-{name}…`
        // → a first segment starting with `<pack>-`.
        let first = rest.split('/').next().unwrap_or("");
        let dashed = first.contains("{name}");
        for planned in pack.plan_install(runtime, files, env)? {
            let shown = planned.path.display();
            if planned
                .path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
            {
                return Err(format!("install.{runtime}: {shown} is not a normal path"));
            }
            let below = planned.path.strip_prefix(&folder).map_err(|_| {
                format!("install.{runtime}: {shown} is outside {}", folder.display())
            })?;
            let mut parts = below.components();
            let head = match parts.next() {
                Some(Component::Normal(h)) => h.to_string_lossy().into_owned(),
                _ => return Err(format!("install.{runtime}: {shown} has no file name")),
            };
            let ok = if dashed {
                head.starts_with(&format!("{}-", pack.name))
            } else {
                head == pack.name && parts.next().is_some()
            };
            if !ok {
                let want = if dashed {
                    format!("{}-…", pack.name)
                } else {
                    format!("{}/…", pack.name)
                };
                return Err(format!(
                    "install.{runtime}: {shown} must be {want} under {}",
                    folder.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(home: &str) -> impl Fn(&str) -> Option<String> {
        let home = home.to_string();
        move |k| (k == "HOME").then(|| home.clone())
    }

    #[test]
    fn placeholders() {
        assert_eq!(
            render_file("a {{ x }} b {{y}}", &[("x", "1"), ("y", "2")]).unwrap(),
            "a 1 b 2"
        );
        assert!(render_file("{{nope}}", &[]).unwrap_err().contains("nope"));
        assert!(render_file("{{x", &[("x", "1")]).is_err());
    }

    #[test]
    fn destinations() {
        let e = |k: &str| match k {
            "HOME" => Some("/home/u".to_string()),
            "CFG" => Some("/cfg".to_string()),
            _ => None,
        };
        assert_eq!(expand("${CFG:-~/.c}/x", &e).unwrap(), "/cfg/x");
        assert_eq!(expand("${NOPE:-~/.c}/x", &e).unwrap(), "/home/u/.c/x");
        assert!(expand("${NOPE}/x", &e).is_err());
        assert_eq!(glob_name("commands/*.md", "commands/next.md"), Some("next"));
        assert_eq!(glob_name("commands/*.md", "FORMAT.md"), None);
        assert_eq!(glob_name("commands/*.md", "commands/a/b.md"), None);
    }

    #[test]
    fn xherd_for_claude_code() {
        let p = crate::builtin();
        let plan = p
            .plan_install("claude-code", BUILTIN_FILES, &env("/home/u"))
            .unwrap();
        let dir = "/home/u/.claude/commands/xherd";
        let next = plan
            .iter()
            .find(|f| f.path == std::path::Path::new(&format!("{dir}/next.md")))
            .unwrap();
        assert!(next.text.contains(&format!("{dir}/rules.md")));
        assert!(!next.text.contains("{{"));
        let rules = plan.iter().find(|f| f.path.ends_with("rules.md")).unwrap();
        assert!(rules.text.contains("## tasks.md") && rules.text.contains("Exit"));
        let feature = plan
            .iter()
            .find(|f| f.path.ends_with("feature.md"))
            .unwrap();
        assert!(feature.text.contains("`$ARGUMENTS`"));
        assert!(p.plan_install("aider", BUILTIN_FILES, &env("/h")).is_err());
    }

    #[test]
    fn slash_commands_are_prefixed_and_sorted() {
        let p = crate::builtin();
        let cc = p.slash_commands("claude-code", BUILTIN_FILES);
        assert!(cc.contains(&"/xherd:next".to_string()));
        assert!(cc.contains(&"/xherd:feature".to_string()));
        assert!(!cc.iter().any(|c| c.contains("FORMAT")));
        let mut sorted = cc.clone();
        sorted.sort();
        assert_eq!(cc, sorted);
        let oc = p.slash_commands("opencode", BUILTIN_FILES);
        assert!(oc.contains(&"/xherd/next".to_string()));
        assert!(p.slash_commands("aider", BUILTIN_FILES).is_empty());
    }

    /// Every command the manifest can send exists as a command file.
    #[test]
    fn every_template_has_a_prompt() {
        let p = crate::builtin();
        assert_eq!(missing_prompts(&p, BUILTIN_FILES), Vec::<String>::new());
    }

    fn mini_pack(next: &str) -> Pack {
        Pack::parse(&format!(
            "name: acme\nversion: 1.0.0\nprotocol: 1\ninstall:\n  claude-code:\n    prefix: '/acme:'\n    commands:\n      from: 'commands/*.md'\n      to: '${{HOME}}/.claude/commands/acme/{{name}}.md'\n      format: command\nflows:\n  f:\n    start: '{{prefix}}spec'\n    stages:\n      - id: spec\n        role: spec\n        class: critical\n        gate:\n          docs: [spec.md]\n        next:\n          - run: '{next}'\n"
        ))
        .unwrap()
    }

    /// A pack `acme` with one install entry per `(runtime, prefix, to, format)`.
    fn acme(installs: &[(&str, &str, &str, &str)]) -> Pack {
        let mut y = String::from("name: acme\nversion: 1.0.0\nprotocol: 1\ninstall:\n");
        for (rt, prefix, to, format) in installs {
            y.push_str(&format!(
                "  {rt}:\n    prefix: '{prefix}'\n    commands:\n      from: 'commands/*.md'\n      to: '{to}'\n      format: {format}\n"
            ));
        }
        y.push_str("flows:\n  f:\n    start: '{prefix}spec'\n    stages:\n      - id: spec\n        role: spec\n        class: critical\n        gate:\n          docs: [spec.md]\n        next:\n          - run: '{prefix}spec'\n");
        Pack::parse(&y).unwrap()
    }

    const ACME_FILES: &[(&str, &str)] =
        &[("commands/spec.md", "---\ndescription: spec\n---\n# spec\n")];

    fn acme_check(p: &Pack, home: &str) -> Result<(), String> {
        check_plugin_install(p, &crate::builtin(), ACME_FILES, &env(home))
    }

    #[test]
    fn runtime_folder_cuts_the_template_not_the_path() {
        let b = crate::builtin();
        let f = |home: &str, rt: &str| runtime_folder(&b, rt, &env(home)).unwrap();
        let home = "/home/xherd";
        assert_eq!(
            f(home, "claude-code"),
            Path::new("/home/xherd/.claude/commands")
        );
        assert_eq!(
            f(home, "opencode"),
            Path::new("/home/xherd/.config/opencode/command")
        );
        assert_eq!(f(home, "codex"), Path::new("/home/xherd/.codex/skills"));
        assert_eq!(f(home, "pi"), Path::new("/home/xherd/.pi/agent/prompts"));
        assert!(runtime_folder(&b, "aider", &env(home)).is_err());
    }

    #[test]
    fn check_plugin_install_accepts_the_builtin_shape() {
        let p = acme(&[
            (
                "claude-code",
                "/acme:",
                "${CLAUDE_CONFIG_DIR:-~/.claude}/commands/acme/{name}.md",
                "command",
            ),
            (
                "codex",
                "$acme-",
                "${CODEX_HOME:-~/.codex}/skills/acme-{name}/SKILL.md",
                "skill",
            ),
            (
                "pi",
                "/acme-",
                "${PI_CODING_AGENT_DIR:-~/.pi/agent}/prompts/acme-{name}.md",
                "command",
            ),
        ]);
        assert_eq!(acme_check(&p, "/home/xherd"), Ok(()));
        assert_eq!(acme_check(&p, "/home/u"), Ok(()));
    }

    #[test]
    fn check_plugin_install_refuses_paths_outside_the_pack_folder() {
        let cc = |prefix: &str, to: &str| acme(&[("claude-code", prefix, to, "command")]);
        let good = "${HOME}/.claude/commands/acme/{name}.md";
        assert_eq!(acme_check(&cc("/acme:", good), "/h"), Ok(()));
        for (prefix, to) in [
            ("/acme:", "${HOME}/.ssh/{name}"),
            ("/acme:", "${HOME}/.claude/commands/acme/../../x/{name}.md"),
            ("/acme:", "${HOME}/.claude/commands/other/{name}.md"),
            ("/acme:", "${HOME}/.claude/commands/{name}.md"),
            ("/acme:", "${HOME}/.claude/commands/acme-x/{name}.md"),
            ("/x:", good),
        ] {
            let e = acme_check(&cc(prefix, to), "/h").unwrap_err();
            assert!(e.starts_with("install.claude-code:"), "{to}: {e}");
        }
        // pi reads one level deep: a sub-folder would never be found.
        let pi = acme(&[(
            "pi",
            "/acme-",
            "${HOME}/.pi/agent/prompts/acme/{name}.md",
            "command",
        )]);
        assert!(acme_check(&pi, "/h").is_err());
        // a skill folder must start with `<pack>-`.
        let codex = acme(&[(
            "codex",
            "$acme-",
            "${HOME}/.codex/skills/other-{name}/SKILL.md",
            "skill",
        )]);
        assert!(acme_check(&codex, "/h").is_err());
        let unknown = acme(&[(
            "aider",
            "/acme:",
            "${HOME}/.aider/acme/{name}.md",
            "command",
        )]);
        assert!(acme_check(&unknown, "/h").unwrap_err().contains("aider"));
    }

    #[test]
    fn missing_prompts_lists_the_command_without_a_file() {
        let p = mini_pack("{prefix}plan");
        let files = [("commands/spec.md", "x")];
        assert_eq!(
            missing_prompts(&p, &files),
            ["{prefix}plan: no commands/plan.md"]
        );
        let files = [("commands/spec.md", "x"), ("commands/plan.md", "y")];
        assert!(missing_prompts(&p, &files).is_empty());
    }

    #[test]
    fn read_pack_dir_reads_format_and_matching_files() {
        let d = &std::env::temp_dir().join(format!("xherd-read-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(d);
        std::fs::create_dir_all(d.join("commands")).unwrap();
        std::fs::write(d.join("FORMAT.md"), "f").unwrap();
        std::fs::write(d.join("commands/spec.md"), "s").unwrap();
        std::fs::write(d.join("commands/notes.txt"), "n").unwrap();
        std::fs::write(d.join("other.md"), "o").unwrap();
        std::fs::create_dir(d.join("commands/dir.md")).unwrap();
        let p = mini_pack("{prefix}spec");
        let files = read_pack_dir(d, &p).unwrap();
        assert_eq!(
            files,
            [
                ("FORMAT.md".to_string(), "f".to_string()),
                ("commands/spec.md".to_string(), "s".to_string())
            ]
        );
        std::fs::write(d.join("commands/bad.md"), [0xff, 0xfe]).unwrap();
        assert!(read_pack_dir(d, &p).unwrap_err().contains("bad.md"));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn read_pack_dir_refuses_a_from_outside_the_pack() {
        let d = &std::env::temp_dir().join(format!("xherd-read-from-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(d);
        std::fs::create_dir_all(d.join("pack")).unwrap();
        std::fs::write(d.join("outside.md"), "secret").unwrap();
        for from in ["../*.md", "commands/../../*.md", "/etc/*.md"] {
            let mut p = mini_pack("{prefix}spec");
            for inst in p.install.values_mut() {
                inst.commands.as_mut().unwrap().from = from.to_string();
            }
            let e = read_pack_dir(&d.join("pack"), &p).unwrap_err();
            assert!(e.contains("relative path"), "{from}: {e}");
        }
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn built_in_files_read_back_from_disk() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/xherd");
        let files = read_pack_dir(&dir, &crate::builtin()).unwrap();
        let mut disk: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        disk.sort();
        let mut embedded: Vec<&str> = BUILTIN_FILES.iter().map(|(p, _)| *p).collect();
        embedded.sort();
        assert_eq!(disk, embedded);
    }

    #[test]
    fn every_pack_file_is_embedded() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/xherd");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir.join("commands"))
            .unwrap()
            .map(|e| format!("commands/{}", e.unwrap().file_name().to_string_lossy()))
            .collect();
        on_disk.push("FORMAT.md".into());
        on_disk.sort();
        let mut embedded: Vec<&str> = BUILTIN_FILES.iter().map(|(p, _)| *p).collect();
        embedded.sort();
        assert_eq!(on_disk, embedded);
    }

    /// The `tasks.md` example in FORMAT.md is what the parser reads.
    #[test]
    fn format_tasks_example_parses() {
        let format = BUILTIN_FILES[0].1;
        let start = format.find("## tasks.md").unwrap();
        let block = format[start..]
            .split("```markdown\n")
            .nth(1)
            .and_then(|b| b.split("\n```").next())
            .unwrap();
        let tasks = crate::tasks::parse(block).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].files, ["src/store.rs", "src/lib.rs"]);
        assert_eq!(tasks[1].depends, ["T01"]);
        assert!(crate::frontmatter::parse(block).unwrap().is_some());
    }
}
