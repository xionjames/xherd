//! `xherd agent …` (issue #141, R20, R22): saved agents over the API.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use crate::Client;

#[derive(Subcommand)]
pub enum AgentCmd {
    /// The job's agent memory (called from a job, XAP).
    #[command(subcommand)]
    Memory(crate::xap::AgentMemoryCmd),
    /// Propose the agent a job without one could become (called from a job, XAP).
    Propose(crate::xap::AgentProposeArgs),
    /// List saved agents.
    Ls,
    /// Show one agent's settings.
    Show { agent: String },
    /// Save a new agent.
    New {
        /// Slug like a job's name; fixed after creation.
        name: String,
        #[command(flatten)]
        fields: Fields,
    },
    /// Change an agent (a new version): only the flags given change.
    Edit {
        agent: String,
        #[command(flatten)]
        fields: Fields,
    },
    /// Hide an agent from pickers (its jobs and versions stay).
    Archive { agent: String },
    /// Bring an archived agent back.
    Unarchive { agent: String },
    /// Delete an agent that has no live jobs.
    Rm { agent: String },
    /// List the jobs started from an agent.
    Jobs { agent: String },
    /// Print agents as JSON (all, or one), for `xherd agent import`.
    Export {
        agent: Option<String>,
        /// Write to this file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Add the agents of an export file: shows every setting first, all or nothing.
    Import {
        file: PathBuf,
        /// Do not ask after showing the settings.
        #[arg(long)]
        yes: bool,
    },
    /// Start a job from an agent (the name defaults to <agent>-<ticket key> or <agent>-<n>).
    Start {
        agent: String,
        prompt: Option<String>,
        #[arg(long)]
        ticket: Option<String>,
        /// Repository (path or URL); default: the current directory.
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        base: Option<String>,
        /// Job name.
        #[arg(long)]
        name: Option<String>,
    },
}

/// What `new` and `edit` can set. An empty value clears a text field, the
/// model of the runtime, or a list.
#[derive(Args, Default)]
pub struct Fields {
    /// One line.
    #[arg(long)]
    description: Option<String>,
    /// Runtime plugin (default for a new agent: claude-code).
    #[arg(long)]
    runtime: Option<String>,
    /// Runtime a job uses when `--runtime`'s is switched off ("" clears).
    #[arg(long)]
    fallback_runtime: Option<String>,
    /// Model for the agent's runtime.
    #[arg(long)]
    model: Option<String>,
    /// What xherd may approve by itself: ask or important.
    #[arg(long)]
    autonomy: Option<xherd_core::autonomy::Autonomy>,
    /// Tool permissions: ask, edits, auto or all (container only).
    #[arg(long)]
    mode: Option<xherd_core::permission::PermissionMode>,
    /// Text added to every job's instructions.
    #[arg(long, conflicts_with = "instructions_file")]
    instructions: Option<String>,
    /// Read the instructions from a file.
    #[arg(long)]
    instructions_file: Option<PathBuf>,
    /// Tokens one job may use (0 = off).
    #[arg(long)]
    job_token_budget: Option<u64>,
    /// Extra sensitive path pattern (repeat; replaces the list; "" clears).
    #[arg(long = "sensitive-path")]
    sensitive_paths: Vec<String>,
    /// Hold a commit over this many changed lines.
    #[arg(long)]
    max_diff_lines: Option<usize>,
    /// A skill of the library (repeat; replaces the list; "" clears).
    #[arg(long = "skill")]
    skills: Vec<String>,
    /// Live jobs of this agent at once (at least 1).
    #[arg(long)]
    max_live_jobs: Option<usize>,
}

/// `["", …]` clears a list; else the values are the list.
fn list(v: &[String]) -> Vec<String> {
    v.iter().filter(|s| !s.is_empty()).cloned().collect()
}

/// Set `key` to the text, or remove it when the text is empty.
fn set_text(m: &mut Map<String, Value>, key: &str, v: &str) {
    if v.is_empty() {
        m.remove(key);
    } else {
        m.insert(key.into(), json!(v));
    }
}

/// Apply the flags to an agent as the API shows it (pure; `new` starts from
/// the defaults, `edit` from the saved agent with the derived fields removed).
pub fn apply(def: &mut Map<String, Value>, f: &Fields, instructions: Option<String>) {
    for k in ["status", "broken", "liveJobs", "lastJob", "tokensToday"] {
        def.remove(k);
    }
    if let Some(v) = &f.description {
        def.insert("description".into(), json!(v));
    }
    if let Some(v) = &f.runtime {
        def.insert("runtime".into(), json!(v));
    }
    if let Some(v) = &f.fallback_runtime {
        set_text(def, "fallbackRuntime", v);
    }
    if let Some(v) = &f.model {
        let runtime = def["runtime"].as_str().unwrap_or("claude-code").to_string();
        let models = def.entry("models").or_insert_with(|| json!({}));
        if v.is_empty() {
            models.as_object_mut().map(|m| m.remove(&runtime));
        } else {
            models[&runtime] = json!(v);
        }
        if models.as_object().is_some_and(Map::is_empty) {
            def.remove("models");
        }
    }
    if let Some(v) = f.autonomy {
        def.insert("autonomy".into(), json!(v));
    }
    if let Some(v) = f.mode {
        def.insert("permissionMode".into(), json!(v));
    }
    if let Some(v) = instructions {
        set_text(def, "instructions", &v);
    }
    if let Some(v) = f.job_token_budget {
        def.insert("jobTokenBudget".into(), json!(v));
    }
    if !f.sensitive_paths.is_empty() {
        set_list(def, "sensitivePaths", list(&f.sensitive_paths));
    }
    if let Some(v) = f.max_diff_lines {
        def.insert("maxDiffLines".into(), json!(v));
    }
    if !f.skills.is_empty() {
        set_list(def, "skills", list(&f.skills));
    }
    if let Some(v) = f.max_live_jobs {
        def.insert("maxLiveJobs".into(), json!(v));
    }
}

fn set_list(m: &mut Map<String, Value>, key: &str, v: Vec<String>) {
    if v.is_empty() {
        m.remove(key);
    } else {
        m.insert(key.into(), json!(v));
    }
}

/// The instructions from the flag or the file.
fn instructions_of(f: &Fields) -> Result<Option<String>, String> {
    match &f.instructions_file {
        Some(p) => std::fs::read_to_string(p)
            .map(Some)
            .map_err(|e| format!("{}: {e}", p.display())),
        None => Ok(f.instructions.clone()),
    }
}

/// Every setting, one per line; a risky one is marked.
pub fn describe(agent: &Value, risky: &[String]) -> String {
    let mut out = String::new();
    for (k, v) in agent.as_object().into_iter().flatten() {
        if matches!(k.as_str(), "id" | "createdAt" | "updatedAt" | "archivedAt") {
            continue;
        }
        let mark = if risky.iter().any(|r| r == k) {
            "!"
        } else {
            " "
        };
        let text = match v {
            Value::String(s) if s.contains('\n') => {
                format!("\n      {}", s.replace('\n', "\n      "))
            }
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        out.push_str(&format!("{mark} {k}: {text}\n"));
    }
    out
}

/// The agents table of `xherd agent ls`.
pub fn table(agents: &[Value]) -> String {
    let mut rows =
        vec![["AGENT", "STATUS", "RUNTIME", "FALLBACK", "LIVE", "LAST JOB"].map(String::from)];
    for a in agents {
        rows.push([
            format!(
                "{} v{}",
                a["name"].as_str().unwrap_or("?"),
                a["version"].as_u64().unwrap_or(1)
            ),
            a["status"].as_str().unwrap_or("?").to_string(),
            a["runtime"].as_str().unwrap_or("?").to_string(),
            a["fallbackRuntime"].as_str().unwrap_or("-").to_string(),
            a["liveJobs"].as_u64().unwrap_or(0).to_string(),
            a["lastJob"].as_str().unwrap_or("-").to_string(),
        ]);
    }
    let width = |i: usize| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0);
    let w: Vec<usize> = (0..5).map(width).collect();
    rows.iter()
        .map(|r| {
            format!(
                "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}  {}\n",
                r[0],
                r[1],
                r[2],
                r[3],
                r[4],
                r[5],
                w0 = w[0],
                w1 = w[1],
                w2 = w[2],
                w3 = w[3],
                w4 = w[4]
            )
        })
        .collect()
}

fn fail(what: &str, e: impl std::fmt::Display) -> i32 {
    eprintln!("xherd agent {what}: {e}");
    2
}

pub fn run(cmd: AgentCmd, json: bool) -> i32 {
    // `memory` is a job's XAP call: it has no user token, only its own.
    if let AgentCmd::Memory(m) = cmd {
        return crate::xap::run(m.into(), json);
    }
    if let AgentCmd::Propose(p) = cmd {
        return crate::xap::run(p.into(), json);
    }
    let c = match Client::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    match cmd {
        AgentCmd::Memory(m) => crate::xap::run(m.into(), json),
        AgentCmd::Propose(p) => crate::xap::run(p.into(), json),
        AgentCmd::Ls => match c.call("GET", "/agents", None) {
            Ok(v) if json => print_json(&v),
            Ok(v) => {
                let a = v["agents"].as_array().cloned().unwrap_or_default();
                if a.is_empty() {
                    println!("no agents");
                } else {
                    print!("{}", table(&a));
                }
                0
            }
            Err(e) => fail("ls", e),
        },
        AgentCmd::Show { agent } => match c.call("GET", &format!("/agents/{agent}"), None) {
            Ok(v) if json => print_json(&v),
            Ok(v) => {
                println!("{}", v["name"].as_str().unwrap_or(&agent));
                print!("{}", describe(&v, &[]));
                0
            }
            Err(e) => fail("show", e),
        },
        AgentCmd::New { name, fields } => {
            let ins = match instructions_of(&fields) {
                Ok(i) => i,
                Err(e) => return fail("new", e),
            };
            let mut def = Map::new();
            def.insert("name".into(), json!(name));
            def.insert("runtime".into(), json!("claude-code"));
            apply(&mut def, &fields, ins);
            saved(
                "new",
                c.call("POST", "/agents", Some(Value::Object(def))),
                json,
            )
        }
        AgentCmd::Edit { agent, fields } => {
            let ins = match instructions_of(&fields) {
                Ok(i) => i,
                Err(e) => return fail("edit", e),
            };
            let mut def = match c.call("GET", &format!("/agents/{agent}"), None) {
                Ok(Value::Object(m)) => m,
                Ok(_) => return fail("edit", "unexpected answer"),
                Err(e) => return fail("edit", e),
            };
            apply(&mut def, &fields, ins);
            saved(
                "edit",
                c.call("PUT", &format!("/agents/{agent}"), Some(Value::Object(def))),
                json,
            )
        }
        AgentCmd::Archive { agent } => saved(
            "archive",
            c.call("POST", &format!("/agents/{agent}/archive"), None),
            json,
        ),
        AgentCmd::Unarchive { agent } => saved(
            "unarchive",
            c.call("POST", &format!("/agents/{agent}/unarchive"), None),
            json,
        ),
        AgentCmd::Rm { agent } => match c.call("DELETE", &format!("/agents/{agent}"), None) {
            Ok(_) => {
                println!("deleted {agent}");
                0
            }
            Err(e) => fail("rm", e),
        },
        AgentCmd::Jobs { agent } => match c.call("GET", &format!("/agents/{agent}/jobs"), None) {
            Ok(v) if json => print_json(&v),
            Ok(v) => {
                let jobs = v["jobs"].as_array().cloned().unwrap_or_default();
                if jobs.is_empty() {
                    println!("no jobs");
                }
                for j in jobs {
                    println!(
                        "{}  {}",
                        j["name"].as_str().unwrap_or("?"),
                        j["status"].as_str().unwrap_or("?")
                    );
                }
                0
            }
            Err(e) => fail("jobs", e),
        },
        AgentCmd::Export { agent, out } => {
            let path = match &agent {
                Some(a) => format!("/agents/export?name={a}"),
                None => "/agents/export".to_string(),
            };
            match c.call("GET", &path, None) {
                Ok(v) => {
                    let text = serde_json::to_string_pretty(&v).unwrap_or_default();
                    match out {
                        Some(p) => match std::fs::write(&p, format!("{text}\n")) {
                            Ok(()) => {
                                println!("wrote {}", p.display());
                                0
                            }
                            Err(e) => fail("export", format!("{}: {e}", p.display())),
                        },
                        None => {
                            println!("{text}");
                            0
                        }
                    }
                }
                Err(e) => fail("export", e),
            }
        }
        AgentCmd::Import { file, yes } => import(&c, &file, yes),
        AgentCmd::Start {
            agent,
            prompt,
            ticket,
            repo,
            base,
            name,
        } => start(&c, &agent, prompt, ticket, repo, base, name),
    }
}

fn print_json(v: &Value) -> i32 {
    println!("{v}");
    0
}

/// One line for an agent a call returned.
fn saved(what: &str, r: Result<Value, String>, json: bool) -> i32 {
    match r {
        Ok(v) if json => print_json(&v),
        Ok(v) => {
            println!(
                "{} v{} ({})",
                v["name"].as_str().unwrap_or("?"),
                v["version"].as_u64().unwrap_or(1),
                v["status"].as_str().unwrap_or("?")
            );
            if let Some(b) = v["broken"].as_str() {
                println!("jobs cannot start from it yet: {b}");
            }
            0
        }
        Err(e) => fail(what, e),
    }
}

/// R22: show every setting and the risky ones, then (after a yes) import.
fn import(c: &Client, file: &std::path::Path, yes: bool) -> i32 {
    let body: Value = match std::fs::read_to_string(file)
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
    {
        Ok(b) => b,
        Err(e) => return fail("import", format!("{}: {e}", file.display())),
    };
    let dry = match c.call("POST", "/agents/import?dryRun=1", Some(body.clone())) {
        Ok(v) => v,
        Err(e) => return fail("import", e),
    };
    let items = dry["agents"].as_array().cloned().unwrap_or_default();
    for it in &items {
        let a = &it["agent"];
        let from_daemon: Vec<String> = it["risky"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(String::from))
            .collect();
        let r = from_daemon;
        println!("{}", a["name"].as_str().unwrap_or("?"));
        print!("{}", describe(a, &r));
        let ignored: Vec<&str> = it["ignored"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str())
            .collect();
        if !ignored.is_empty() {
            println!(
                "  ignored (agents no longer have them): {}",
                ignored.join(", ")
            );
        }
        if it["nameTaken"].as_bool().unwrap_or(false) {
            println!("  ! an agent with this name exists: the import will be refused");
        }
        if !r.is_empty() {
            println!("  ! look at: {}", r.join(", "));
        }
        println!();
    }
    if dry["nameTaken"].as_bool().unwrap_or(false) {
        return fail("import", "name_taken: nothing imported");
    }
    if !yes {
        print!("Import {} agent(s)? [y/N] ", items.len());
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        if !matches!(line.trim(), "y" | "Y" | "yes") {
            println!("nothing imported");
            return 1;
        }
    }
    match c.call("POST", "/agents/import", Some(body)) {
        Ok(v) => {
            println!("imported {}", v["added"].as_u64().unwrap_or(0));
            0
        }
        Err(e) => fail("import", e),
    }
}

fn start(
    c: &Client,
    agent: &str,
    prompt: Option<String>,
    ticket: Option<String>,
    repo: Option<String>,
    base: Option<String>,
    name: Option<String>,
) -> i32 {
    // The daemon has its own working directory: send an absolute path.
    let repo = match repo {
        Some(r) if r.contains("://") || r.starts_with("git@") => r,
        Some(r) => match std::fs::canonicalize(&r) {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(e) => return fail("start", format!("repository: {e}")),
        },
        None => match std::env::current_dir() {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(e) => return fail("start", format!("repository: {e}")),
        },
    };
    let body = json!({
        "name": name.unwrap_or_default(),
        "ticket": ticket,
        "repo": repo,
        "base": base,
        "prompt": prompt,
    });
    match c.call("POST", &format!("/agents/{agent}/jobs"), Some(body)) {
        Ok(v) => {
            let j = &v["job"];
            println!(
                "{} {} in {}",
                j["name"].as_str().unwrap_or("?"),
                j["status"].as_str().unwrap_or("?"),
                j["workdir"].as_str().unwrap_or("?")
            );
            0
        }
        Err(e) => fail("start", e),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct T {
        #[command(subcommand)]
        c: AgentCmd,
    }

    fn parse(args: &[&str]) -> AgentCmd {
        T::try_parse_from(std::iter::once("agent").chain(args.iter().copied()))
            .unwrap()
            .c
    }

    #[test]
    fn every_subcommand_parses() {
        for a in [
            &["ls"][..],
            &["show", "dev"],
            &[
                "new",
                "dev",
                "--runtime",
                "codex",
                "--fallback-runtime",
                "opencode",
            ],
            &[
                "new",
                "dev",
                "--model",
                "opus",
                "--autonomy",
                "important",
                "--mode",
                "edits",
            ],
            &[
                "new",
                "dev",
                "--sensitive-path",
                "a/**",
                "--sensitive-path",
                "b",
                "--skill",
                "s",
                "--max-live-jobs",
                "2",
            ],
            &[
                "edit",
                "dev",
                "--description",
                "x",
                "--job-token-budget",
                "5",
                "--max-diff-lines",
                "9",
            ],
            &["archive", "dev"],
            &["unarchive", "dev"],
            &["rm", "dev"],
            &["jobs", "dev"],
            &["export"],
            &["export", "dev", "--out", "a.json"],
            &["import", "a.json"],
            &["import", "a.json", "--yes"],
            &["start", "dev"],
            &[
                "start", "dev", "Fix it", "--ticket", "SC-1", "--repo", ".", "--name", "x",
                "--base", "main",
            ],
        ] {
            parse(a);
        }
        assert!(
            T::try_parse_from([
                "agent",
                "new",
                "dev",
                "--instructions",
                "a",
                "--instructions-file",
                "f"
            ])
            .is_err()
        );
        assert!(T::try_parse_from(["agent", "show"]).is_err());
    }

    fn fields(args: &[&str]) -> Fields {
        match parse(&[&["new", "x"], args].concat()) {
            AgentCmd::New { fields, .. } => fields,
            _ => unreachable!(),
        }
    }

    #[test]
    fn new_starts_from_the_defaults_and_edit_changes_only_what_is_given() {
        let f = fields(&[
            "--model",
            "opus",
            "--mode",
            "edits",
            "--fallback-runtime",
            "opencode",
        ]);
        let mut d = Map::new();
        d.insert("name".into(), json!("x"));
        d.insert("runtime".into(), json!("claude-code"));
        apply(&mut d, &f, None);
        assert_eq!(
            Value::Object(d.clone()),
            json!({"name": "x", "runtime": "claude-code",
                "fallbackRuntime": "opencode",
                "models": {"claude-code": "opus"}, "permissionMode": "edits"})
        );
        // An edit of a saved agent: derived fields go, the rest stays.
        let mut saved = d;
        saved.insert("status".into(), json!("idle"));
        saved.insert("liveJobs".into(), json!(0));
        saved.insert("version".into(), json!(3));
        apply(
            &mut saved,
            &fields(&["--fallback-runtime", "codex", "--model", ""]),
            None,
        );
        assert_eq!(saved["fallbackRuntime"], json!("codex"));
        assert!(!saved.contains_key("models") && !saved.contains_key("status"));
        assert_eq!(saved["version"], json!(3));
        assert_eq!(saved["permissionMode"], json!("edits"));
        // Empty values clear.
        apply(
            &mut saved,
            &fields(&["--fallback-runtime", ""]),
            Some("x".into()),
        );
        assert!(!saved.contains_key("fallbackRuntime"));
        assert_eq!(saved["instructions"], json!("x"));
        apply(&mut saved, &fields(&[]), Some(String::new()));
        assert!(!saved.contains_key("instructions"));
    }

    #[test]
    fn a_model_follows_a_changed_runtime() {
        let mut d = Map::new();
        d.insert("runtime".into(), json!("claude-code"));
        apply(
            &mut d,
            &fields(&["--runtime", "codex", "--model", "gpt"]),
            None,
        );
        assert_eq!(d["models"], json!({"codex": "gpt"}));
    }

    #[test]
    fn import_marks_what_widens_or_instructs() {
        let a = json!({"name": "a", "permissionMode": "all",
            "instructions": "line 1\nline 2"});
        let r: Vec<String> = vec!["permissionMode".into(), "instructions".into()];
        let text = describe(&a, &r);
        assert!(text.contains("! permissionMode: all\n"), "{text}");
        assert!(text.contains("  name: a\n"), "{text}");
        assert!(
            text.contains("! instructions: \n      line 1\n      line 2\n"),
            "{text}"
        );
    }

    #[test]
    fn the_table_aligns_its_columns() {
        let a = [
            json!({"name": "reviewer", "version": 2, "status": "running", "runtime": "claude-code",
                "fallbackRuntime": "opencode", "liveJobs": 1, "lastJob": "reviewer-3"}),
            json!({"name": "b", "status": "idle", "runtime": "codex"}),
        ];
        assert_eq!(
            table(&a),
            "AGENT        STATUS   RUNTIME      FALLBACK  LIVE  LAST JOB\n\
             reviewer v2  running  claude-code  opencode  1     reviewer-3\n\
             b v1         idle     codex        -         0     -\n"
        );
    }
}
