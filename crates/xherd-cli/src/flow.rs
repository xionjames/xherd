//! `xherd flow …` (issue #141, D26, R35): which saved agent runs which stage
//! of a workflow, over `/api/v1/flows`.

use clap::Subcommand;
use serde_json::{Value, json};

use crate::Client;

/// The pack a bare flow name belongs to.
const DEFAULT_PACK: &str = "xherd";

#[derive(Subcommand)]
pub enum FlowCmd {
    /// List the flows with how many of their stages have an agent.
    Ls,
    /// Show a flow's stages and the agent each one resolves to.
    Show {
        /// `feature`, or `pack/flow` for a flow of another pack.
        flow: String,
    },
    /// Give a stage to a saved agent (replaces the stage's assignment).
    Assign {
        flow: String,
        stage: String,
        agent: String,
        /// The stage's gate always waits for you, whatever the autonomy.
        #[arg(long, conflicts_with = "no_human")]
        human: bool,
        /// Clear the human flag (the default when the stage is new).
        #[arg(long)]
        no_human: bool,
    },
    /// Take a stage back from its agent: it uses the job's agent, else the
    /// stage before.
    Unassign { flow: String, stage: String },
}

/// `feature` or `pack/flow`.
pub fn split_flow(s: &str) -> (&str, &str) {
    s.split_once('/').unwrap_or((DEFAULT_PACK, s))
}

/// The assignment rows with `stage` given to `agent`. `human` None keeps the
/// stage's current flag.
pub fn assigned(rows: &[Value], stage: &str, agent: &str, human: Option<bool>) -> Vec<Value> {
    let old = rows.iter().find(|r| r["stage"] == stage);
    let human = human.unwrap_or_else(|| old.is_some_and(|r| r["human"] == json!(true)));
    let mut out: Vec<Value> = rows
        .iter()
        .filter(|r| r["stage"] != stage)
        .cloned()
        .collect();
    out.push(json!({"stage": stage, "agentId": agent, "human": human}));
    out
}

/// The rows without `stage`, or `None` when it had none.
pub fn unassigned(rows: &[Value], stage: &str) -> Option<Vec<Value>> {
    rows.iter().any(|r| r["stage"] == stage).then(|| {
        rows.iter()
            .filter(|r| r["stage"] != stage)
            .cloned()
            .collect()
    })
}

fn pad(rows: Vec<Vec<String>>) -> String {
    let cols = rows.first().map_or(0, Vec::len);
    let w: Vec<usize> = (0..cols)
        .map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0))
        .collect();
    rows.iter()
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
        .collect()
}

/// `xherd flow ls`: one line per flow.
pub fn flows_table(flows: &[Value]) -> String {
    let mut rows = vec![
        ["FLOW", "STAGES", "ASSIGNED", "AGENTS"]
            .map(String::from)
            .to_vec(),
    ];
    for f in flows {
        let stages = f["stages"].as_array().cloned().unwrap_or_default();
        let given: Vec<&Value> = stages
            .iter()
            .filter(|s| s["assigned"] == json!(true))
            .collect();
        let mut names: Vec<&str> = given.iter().filter_map(|s| s["agent"].as_str()).collect();
        names.sort();
        names.dedup();
        rows.push(vec![
            format!(
                "{}/{}",
                f["pack"].as_str().unwrap_or("?"),
                f["flow"].as_str().unwrap_or("?")
            ),
            stages.len().to_string(),
            given.len().to_string(),
            if names.is_empty() {
                "-".into()
            } else {
                names.join(", ")
            },
        ]);
    }
    pad(rows)
}

/// `xherd flow show`: a stage per line, its agent, `(same)` when it only
/// follows the stage before, `human` and why the agent cannot run.
pub fn stages_table(flow: &Value) -> String {
    let mut rows = vec![["STAGE", "AGENT", "NOTES"].map(String::from).to_vec()];
    for s in flow["stages"].as_array().into_iter().flatten() {
        let mut notes = Vec::new();
        if s["agent"].is_string() && s["assigned"] != json!(true) {
            notes.push("same as before".to_string());
        }
        if s["human"] == json!(true) {
            notes.push("human".into());
        }
        if let Some(b) = s["broken"].as_str() {
            notes.push(format!("broken: {b}"));
        }
        rows.push(vec![
            s["id"].as_str().unwrap_or("?").to_string(),
            s["agent"].as_str().unwrap_or("-").to_string(),
            notes.join(", "),
        ]);
    }
    pad(rows)
}

fn fail(what: &str, e: impl std::fmt::Display) -> i32 {
    eprintln!("xherd flow {what}: {e}");
    2
}

fn print_json(v: &Value) -> i32 {
    println!("{v}");
    0
}

pub fn run(cmd: FlowCmd, json: bool) -> i32 {
    let c = match Client::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let rows = |c: &Client, pack: &str, flow: &str| -> Result<Vec<Value>, String> {
        let v = c.call("GET", &format!("/flows/{pack}/{flow}/assignments"), None)?;
        Ok(v["assignments"].as_array().cloned().unwrap_or_default())
    };
    let put = |c: &Client, pack: &str, flow: &str, rows: Vec<Value>| {
        c.call(
            "PUT",
            &format!("/flows/{pack}/{flow}/assignments"),
            Some(json!({"assignments": rows})),
        )
    };
    match cmd {
        FlowCmd::Ls => match c.call("GET", "/flows", None) {
            Ok(v) if json => print_json(&v),
            Ok(v) => {
                print!(
                    "{}",
                    flows_table(&v["flows"].as_array().cloned().unwrap_or_default())
                );
                0
            }
            Err(e) => fail("ls", e),
        },
        FlowCmd::Show { flow } => {
            let (pack, name) = split_flow(&flow);
            match c.call("GET", &format!("/flows/{pack}/{name}"), None) {
                Ok(v) if json => print_json(&v),
                Ok(v) => {
                    print!("{}", stages_table(&v));
                    0
                }
                Err(e) => fail("show", e),
            }
        }
        FlowCmd::Assign {
            flow,
            stage,
            agent,
            human,
            no_human,
        } => {
            let (pack, name) = split_flow(&flow);
            let flag = if human {
                Some(true)
            } else if no_human {
                Some(false)
            } else {
                None
            };
            let next = match rows(&c, pack, name) {
                Ok(r) => assigned(&r, &stage, &agent, flag),
                Err(e) => return fail("assign", e),
            };
            match put(&c, pack, name, next) {
                Ok(v) if json => print_json(&v),
                Ok(v) => {
                    print!("{}", stages_table(&v));
                    0
                }
                Err(e) => fail("assign", e),
            }
        }
        FlowCmd::Unassign { flow, stage } => {
            let (pack, name) = split_flow(&flow);
            let next = match rows(&c, pack, name) {
                Ok(r) => match unassigned(&r, &stage) {
                    Some(n) => n,
                    None => return fail("unassign", format!("stage {stage} has no agent")),
                },
                Err(e) => return fail("unassign", e),
            };
            match put(&c, pack, name, next) {
                Ok(v) if json => print_json(&v),
                Ok(v) => {
                    print!("{}", stages_table(&v));
                    0
                }
                Err(e) => fail("unassign", e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct T {
        #[command(subcommand)]
        c: FlowCmd,
    }

    fn parse(args: &[&str]) -> Result<FlowCmd, clap::Error> {
        T::try_parse_from(std::iter::once("flow").chain(args.iter().copied())).map(|t| t.c)
    }

    #[test]
    fn every_subcommand_parses() {
        for a in [
            &["ls"][..],
            &["show", "feature"],
            &["show", "acme/release"],
            &["assign", "feature", "design", "architect"],
            &["assign", "feature", "review", "reviewer", "--human"],
            &["assign", "feature", "review", "reviewer", "--no-human"],
            &["unassign", "feature", "design"],
        ] {
            assert!(parse(a).is_ok(), "{a:?}");
        }
        assert!(parse(&["assign", "f", "s", "a", "--human", "--no-human"]).is_err());
        assert!(parse(&["assign", "f", "s"]).is_err());
    }

    #[test]
    fn a_flow_is_named_with_or_without_its_pack() {
        assert_eq!(split_flow("feature"), ("xherd", "feature"));
        assert_eq!(split_flow("acme/release"), ("acme", "release"));
    }

    #[test]
    fn assigning_replaces_the_stage_and_keeps_its_human_flag_unless_told() {
        let rows = vec![
            json!({"stage": "design", "agentId": "a", "human": false}),
            json!({"stage": "review", "agentId": "b", "human": true}),
        ];
        let kept = assigned(&rows, "review", "c", None);
        assert_eq!(kept.len(), 2);
        assert!(kept.contains(&json!({"stage": "review", "agentId": "c", "human": true})));
        let cleared = assigned(&rows, "review", "c", Some(false));
        assert!(cleared.contains(&json!({"stage": "review", "agentId": "c", "human": false})));
        let added = assigned(&rows, "ship", "c", None);
        assert_eq!(added.len(), 3);
        assert!(added.contains(&json!({"stage": "ship", "agentId": "c", "human": false})));
        assert_eq!(unassigned(&rows, "design").unwrap().len(), 1);
        assert!(unassigned(&rows, "ship").is_none());
    }

    #[test]
    fn the_tables_say_what_each_stage_runs() {
        let flow = json!({"pack": "xherd", "flow": "feature", "stages": [
            {"id": "spec", "assigned": false, "human": false},
            {"id": "design", "agent": "architect", "assigned": true, "human": false},
            {"id": "tasks", "agent": "architect", "assigned": false, "human": false},
            {"id": "review", "agent": "reviewer", "assigned": true, "human": true,
             "broken": "the agent is archived"},
        ]});
        let t = stages_table(&flow);
        let line = |id: &str| t.lines().find(|l| l.starts_with(id)).unwrap().to_string();
        assert!(line("spec").contains('-'));
        assert!(line("tasks").contains("same as before"));
        assert!(line("design").trim_end().ends_with("architect"));
        assert!(line("review").contains("human, broken: the agent is archived"));
        let ls = flows_table(&[flow]);
        assert!(
            ls.contains("xherd/feature") && ls.contains("2") && ls.contains("architect, reviewer")
        );
    }
}
