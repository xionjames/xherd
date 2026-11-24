//! `xherd plugin …`: install, update, remove and list external plugins over
//! `/api/v1/plugins` (R13). Every change shows a preview first; the digest of
//! what was shown is what the daemon installs.

use std::io::{BufRead, IsTerminal, Write};

use clap::Subcommand;
use serde_json::{Value, json};

use crate::Client;

#[derive(Subcommand)]
pub enum PluginCmd {
    /// List the plugins with where each external one came from.
    Ls,
    /// Install an external plugin from a git repository (shows what it is first).
    Install {
        /// `https://…`, `git@host:owner/repo`, `ssh://…` or a GitHub `tree/<ref>/<path>` URL.
        url: String,
        /// Branch, tag or commit (default: the repository's HEAD).
        #[arg(long = "ref")]
        git_ref: Option<String>,
        /// The plugin's folder in the repository, when it holds several.
        #[arg(long)]
        path: Option<String>,
        /// Install without asking (required when there is no terminal).
        #[arg(long, short)]
        yes: bool,
    },
    /// Fetch an installed plugin's source again and apply the change.
    Update {
        name: String,
        /// Move to another branch, tag or commit.
        #[arg(long = "ref")]
        git_ref: Option<String>,
        #[arg(long, short)]
        yes: bool,
    },
    /// Remove an external plugin.
    Remove {
        name: String,
        /// Also drop its settings and secrets.
        #[arg(long)]
        forget: bool,
        #[arg(long, short)]
        yes: bool,
    },
}

fn fail(what: &str, e: impl std::fmt::Display) -> i32 {
    eprintln!("xherd plugin {what}: {e}");
    2
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// `url@sha` of a recorded source, or what a plugin without one is.
fn source_text(p: &Value) -> String {
    if p["builtin"] == json!(true) {
        return "built-in".into();
    }
    match p["source"]["url"].as_str() {
        Some(url) => format!(
            "{url}@{}",
            short(p["source"]["sha"].as_str().unwrap_or("?"))
        ),
        None => "installed by hand, not pinned".into(),
    }
}

/// `xherd plugin ls`: one line per plugin.
pub fn plugins_table(v: &Value) -> String {
    let mut rows = vec![
        ["NAME", "KIND", "VERSION", "SOURCE"]
            .map(String::from)
            .to_vec(),
    ];
    let mut plugins: Vec<&Value> = v["plugins"].as_array().into_iter().flatten().collect();
    plugins.sort_by_key(|p| (p["builtin"] == json!(true), p["name"].as_str()));
    for p in plugins {
        rows.push(vec![
            p["name"].as_str().unwrap_or("?").into(),
            p["kind"].as_str().unwrap_or("?").into(),
            p["version"].as_str().unwrap_or("?").into(),
            source_text(p),
        ]);
    }
    let cols = 4;
    let w: Vec<usize> = (0..cols)
        .map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0))
        .collect();
    let mut out: String = rows
        .iter()
        .map(|r| {
            let line: Vec<String> = r
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    if i + 1 == cols {
                        c.clone()
                    } else {
                        format!("{c:<w$}", w = w[i])
                    }
                })
                .collect();
            format!("{}\n", line.join("  ").trim_end())
        })
        .collect();
    if v["restartNeeded"] == json!(true) {
        out.push_str("\nRestart needed to load the changes: xherd daemon restart\n");
    }
    out
}

/// What a source would install, as plain text.
pub fn preview_text(v: &Value) -> String {
    let s = |k: &str| v[k].as_str().unwrap_or("?");
    let mut o = format!(
        "{} {} ({}) from {}@{}\n",
        s("name"),
        s("version"),
        s("kind"),
        s("url"),
        short(s("sha"))
    );
    if let Some(p) = v["path"].as_str() {
        o.push_str(&format!("  path: {p}\n"));
    }
    if let Some(d) = v["description"].as_str().filter(|d| !d.is_empty()) {
        o.push_str(&format!("  {d}\n"));
    }
    o.push_str(&format!(
        "  {} files, {} bytes, digest {}\n",
        v["files"],
        v["bytes"],
        s("digest")
    ));
    if let Some(r) = v["replaces"].as_str() {
        o.push_str(&format!("  replaces the {r} plugin of the same name\n"));
    }
    if v["compatible"] == json!(false) {
        o.push_str(&format!(
            "  needs xherd {} or newer\n",
            v["minXherdVersion"].as_str().unwrap_or("?")
        ));
    }
    for r in v["runs"].as_array().into_iter().flatten() {
        let argv: Vec<&str> = r["argv"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        o.push_str(&format!(
            "  runs ({}): {}\n",
            r["what"].as_str().unwrap_or("?"),
            argv.join(" ")
        ));
    }
    flow_text(&v["flow"], &mut o);
    if let Some(w) = v["warning"].as_str().filter(|w| !w.is_empty()) {
        o.push_str(&format!("\n{w}\n"));
    }
    o
}

/// The `flow` part of a flow plugin's preview: flows, agents with their
/// state, and the files per runtime.
fn flow_text(f: &Value, o: &mut String) {
    if !f.is_object() {
        return;
    }
    o.push_str(&format!(
        "  flow pack {}\n",
        f["pack"].as_str().unwrap_or("?")
    ));
    for fl in f["flows"].as_array().into_iter().flatten() {
        let stages: Vec<&str> = fl["stages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s["id"].as_str())
            .collect();
        o.push_str(&format!(
            "  flow {}: {}\n",
            fl["flow"].as_str().unwrap_or("?"),
            stages.join(", ")
        ));
    }
    for a in f["agents"].as_array().into_iter().flatten() {
        let risky: Vec<&str> = a["risky"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        o.push_str(&format!(
            "  agent {} ({}){}\n",
            a["savedName"].as_str().unwrap_or("?"),
            a["state"].as_str().unwrap_or("?"),
            if risky.is_empty() {
                String::new()
            } else {
                format!(", risky: {}", risky.join(", "))
            }
        ));
        for i in a["issues"].as_array().into_iter().flatten() {
            o.push_str(&format!("    problem: {}\n", i.as_str().unwrap_or("?")));
        }
    }
    for a in f["archived"].as_array().into_iter().flatten() {
        o.push_str(&format!(
            "  agent {} is archived\n",
            a.as_str().unwrap_or("?")
        ));
    }
    for r in f["files"].as_array().into_iter().flatten() {
        let paths: Vec<&str> = r["paths"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        o.push_str(&format!(
            "  files for {} ({}): {}\n",
            r["runtime"].as_str().unwrap_or("?"),
            paths.len(),
            paths.join(", ")
        ));
    }
    if let Some(u) = f["inUseBy"].as_str() {
        o.push_str(&format!("  in use by job {u}\n"));
    }
}

/// The repository holds several plugins: say which `--path` to give.
pub fn choices_text(v: &Value) -> String {
    let mut o = format!("{} holds several plugins; choose one with --path:\n", {
        v["url"].as_str().unwrap_or("the repository")
    });
    for c in v["choices"].as_array().into_iter().flatten() {
        o.push_str(&format!(
            "  {}  ({}{})\n",
            c["path"].as_str().unwrap_or("?"),
            c["name"].as_str().unwrap_or("?"),
            c["error"]
                .as_str()
                .map(|e| format!(", unusable: {e}"))
                .unwrap_or_default()
        ));
    }
    o
}

/// Ask on the terminal; without `--yes` and without a terminal there is no
/// one to ask, so the answer is no (the caller exits 2).
fn confirm(question: &str, yes: bool) -> Result<bool, String> {
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err("no terminal to ask on: add --yes to go ahead".into());
    }
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

fn restart_hint() {
    println!("Restart xherd to load it: xherd daemon restart");
}

fn install(c: &Client, json: bool, body: Value, yes: bool) -> i32 {
    let pv = match c.call_slow("POST", "/plugins/preview", Some(body.clone())) {
        Ok(v) => v,
        Err(e) => return fail("install", e),
    };
    if pv["choices"].as_array().is_some_and(|c| !c.is_empty()) && pv["name"].is_null() {
        eprint!("{}", choices_text(&pv));
        return 2;
    }
    if !json {
        print!("{}", preview_text(&pv));
    }
    match confirm("Install this plugin?", yes) {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("not installed");
            return 2;
        }
        Err(e) => return fail("install", e),
    }
    let mut b = body;
    b["digest"] = pv["digest"].clone();
    match c.call_slow("POST", "/plugins/install", Some(b)) {
        Ok(v) if json => print_json(&v),
        Ok(v) => {
            println!(
                "Installed {} {} ({}@{}).",
                v["name"].as_str().unwrap_or("?"),
                v["version"].as_str().unwrap_or("?"),
                v["url"].as_str().unwrap_or("?"),
                short(v["sha"].as_str().unwrap_or("?"))
            );
            restart_hint();
            0
        }
        Err(e) => fail("install", e),
    }
}

fn update(c: &Client, json: bool, name: &str, git_ref: Option<String>, yes: bool) -> i32 {
    let path = format!("/plugins/{name}/update");
    let body = json!({"ref": git_ref});
    let pv = match c.call_slow("POST", &path, Some(body.clone())) {
        Ok(v) => v,
        Err(e) => return fail("update", e),
    };
    if !json {
        print!("{}", preview_text(&pv));
        match pv["manifestDiff"].as_str().filter(|d| !d.is_empty()) {
            Some(d) => print!("\nManifest changes:\n{d}"),
            None => println!("\nThe manifest is unchanged."),
        }
    }
    match confirm("Update this plugin?", yes) {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("not updated");
            return 2;
        }
        Err(e) => return fail("update", e),
    }
    let mut b = body;
    b["digest"] = pv["digest"].clone();
    match c.call_slow("POST", &path, Some(b)) {
        Ok(v) if json => print_json(&v),
        Ok(v) => {
            println!(
                "Updated {} to {} ({}).",
                v["name"].as_str().unwrap_or("?"),
                v["version"].as_str().unwrap_or("?"),
                short(v["sha"].as_str().unwrap_or("?"))
            );
            restart_hint();
            0
        }
        Err(e) => fail("update", e),
    }
}

fn remove(c: &Client, json: bool, name: &str, forget: bool, yes: bool) -> i32 {
    match confirm(&format!("Remove plugin {name}?"), yes) {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("not removed");
            return 2;
        }
        Err(e) => return fail("remove", e),
    }
    match c.call("DELETE", &format!("/plugins/{name}?forget={forget}"), None) {
        Ok(v) if json => print_json(&v),
        Ok(v) => {
            println!(
                "Removed {}{}.",
                v["name"].as_str().unwrap_or(name),
                if v["forgot"] == json!(true) {
                    " with its settings and secrets"
                } else {
                    ""
                }
            );
            println!("Restart xherd to unload it: xherd daemon restart");
            0
        }
        Err(e) => fail("remove", e),
    }
}

fn print_json(v: &Value) -> i32 {
    println!("{v}");
    0
}

pub fn run(cmd: PluginCmd, json: bool) -> i32 {
    crate::with_client(|c| match cmd {
        PluginCmd::Ls => match c.call("GET", "/plugins", None) {
            Ok(v) if json => print_json(&v),
            Ok(v) => {
                print!("{}", plugins_table(&v));
                0
            }
            Err(e) => fail("ls", e),
        },
        PluginCmd::Install {
            url,
            git_ref,
            path,
            yes,
        } => install(
            c,
            json,
            json!({"url": url, "ref": git_ref, "path": path}),
            yes,
        ),
        PluginCmd::Update { name, git_ref, yes } => update(c, json, &name, git_ref, yes),
        PluginCmd::Remove { name, forget, yes } => remove(c, json, &name, forget, yes),
    })
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct T {
        #[command(subcommand)]
        c: PluginCmd,
    }

    fn parse(args: &[&str]) -> Result<PluginCmd, clap::Error> {
        T::try_parse_from(std::iter::once("plugin").chain(args.iter().copied())).map(|t| t.c)
    }

    #[test]
    fn preview_prints_the_flow_part() {
        let v = json!({
            "name": "acme-sdd", "version": "1.0.0", "kind": "flow", "url": "u", "sha": "abc",
            "files": 3, "bytes": 10, "digest": "d",
            "flow": {
                "pack": "acme-sdd",
                "flows": [{"flow": "sdd", "stages": [{"id": "spec"}, {"id": "build"}]}],
                "agents": [
                    {"savedName": "acme-sdd-architect", "state": "new", "risky": ["mode"], "issues": []},
                    {"savedName": "acme-sdd-dev", "state": "taken", "risky": [], "issues": ["name taken"]}
                ],
                "archived": ["acme-sdd-old"],
                "files": [{"runtime": "claude-code", "paths": ["a.md", "b.md"]}]
            }
        });
        let t = preview_text(&v);
        assert!(t.contains("  flow sdd: spec, build\n"), "{t}");
        assert!(
            t.contains("  agent acme-sdd-architect (new), risky: mode\n"),
            "{t}"
        );
        assert!(
            t.contains("  agent acme-sdd-dev (taken)\n    problem: name taken\n"),
            "{t}"
        );
        assert!(t.contains("  agent acme-sdd-old is archived\n"), "{t}");
        assert!(
            t.contains("  files for claude-code (2): a.md, b.md\n"),
            "{t}"
        );
        assert!(!preview_text(&json!({"name": "x"})).contains("flow pack"));
    }

    #[test]
    fn every_subcommand_parses() {
        for a in [
            &["ls"][..],
            &["install", "https://github.com/o/r"],
            &[
                "install",
                "https://github.com/o/r",
                "--ref",
                "v1",
                "--path",
                "p",
                "--yes",
            ],
            &["update", "gitea", "--ref", "v2", "-y"],
            &["remove", "gitea", "--forget", "--yes"],
        ] {
            assert!(parse(a).is_ok(), "{a:?}");
        }
        assert!(parse(&["install"]).is_err());
        assert!(parse(&["remove"]).is_err());
    }

    #[test]
    fn the_list_shows_sources_and_the_restart_note() {
        let v = json!({"restartNeeded": true, "plugins": [
            {"name": "github", "kind": "host", "version": "1.0.0", "builtin": true},
            {"name": "gitea", "kind": "host", "version": "0.2.0", "builtin": false,
             "source": {"url": "https://h/o/r", "sha": "0123456789abcdef"}},
            {"name": "hand", "kind": "tracker", "version": "1.0.0", "builtin": false},
        ]});
        let t = plugins_table(&v);
        let line = |n: &str| t.lines().find(|l| l.starts_with(n)).unwrap().to_string();
        assert!(line("gitea").ends_with("https://h/o/r@0123456"));
        assert!(line("hand").ends_with("installed by hand, not pinned"));
        assert!(line("github").ends_with("built-in"));
        assert!(t.contains("xherd daemon restart"));
        assert!(t.find("gitea").unwrap() < t.find("github").unwrap());
    }

    #[test]
    fn choices_name_the_path_to_give() {
        let t = choices_text(&json!({"url": "https://h/o/r", "choices": [
            {"path": "plugins/a", "name": "a", "kind": "host"},
            {"path": "plugins/b", "name": "b", "kind": null, "error": "bad manifest"},
        ]}));
        assert!(t.contains("--path") && t.contains("plugins/a  (a)"));
        assert!(t.contains("plugins/b  (b, unusable: bad manifest)"));
    }
}
