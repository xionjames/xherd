//! Rendering a pack's command files for a runtime (design §6.7). Writing them
//! and keeping user edits is the daemon's job (`xherd install`).

use std::path::PathBuf;

use crate::Pack;

/// The built-in pack's files, by path inside `packs/xherd/`.
pub const BUILTIN_FILES: &[(&str, &str)] = &[
    ("FORMAT.md", include_str!("../../../packs/xherd/FORMAT.md")),
    (
        "commands/commit.md",
        include_str!("../../../packs/xherd/commands/commit.md"),
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
            if set.format != "command" {
                return Err(format!(
                    "install.{runtime}: format {:?} is not supported yet",
                    set.format
                ));
            }
            for (path, text) in files {
                let Some(name) = glob_name(&set.from, path) else {
                    continue;
                };
                let to = crate::template::render(&set.to, &[("name", name)])?;
                let dest = PathBuf::from(expand(&to, env)?);
                let dir = dest
                    .parent()
                    .map(|d| d.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let text = render_file(
                    text,
                    &[
                        ("prefix", &inst.prefix),
                        ("args", inst.args.as_deref().unwrap_or("")),
                        ("dir", &dir),
                        ("xap", XAP_SHELL),
                        ("format", format),
                    ],
                )
                .map_err(|e| format!("{path}: {e}"))?;
                out.push(Planned { path: dest, text });
            }
        }
        Ok(out)
    }
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
        assert!(p.plan_install("codex", BUILTIN_FILES, &env("/h")).is_err());
    }

    /// Every command the manifest can send exists as a command file.
    #[test]
    fn every_template_has_a_prompt() {
        let p = crate::builtin();
        let mut templates: Vec<&str> = p.commands.values().map(String::as_str).collect();
        for f in p.flows.values() {
            templates.push(&f.start);
            for s in &f.stages {
                for r in &s.next {
                    templates.push(&r.run);
                    templates.extend(r.auto_approved.as_deref());
                }
            }
        }
        for t in templates {
            let name = t
                .strip_prefix("{prefix}")
                .and_then(|r| r.split_whitespace().next())
                .unwrap_or_else(|| panic!("{t} does not start with {{prefix}}"));
            let file = format!("commands/{name}.md");
            assert!(
                BUILTIN_FILES.iter().any(|(p, _)| *p == file),
                "{t}: no {file}"
            );
        }
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
