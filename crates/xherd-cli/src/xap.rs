//! XAP commands (xap.md): `xherd gate|ask|stage|tasks|task|commit|pr|handoff|issue`.
//! Synchronous like `report`. Exit codes: 0 accepted, 2 refused, 3 daemon
//! unreachable, 4 not under xherd, 64 usage (clap).

use std::time::Duration;

use clap::Subcommand;
use serde_json::Value;
use xherd_core::domain::Role;
use xherd_core::paths::Paths;
use xherd_core::xap::{Severity, XapCall, XapCmd};

use crate::report::{DEFAULT_URL, find_agent};

fn role(s: &str) -> Result<Role, String> {
    serde_json::from_value(Value::String(s.to_string())).map_err(|_| {
        "one of explore, spec, design, tasks, implement, review, ship, pr, custom".to_string()
    })
}

fn severity(s: &str) -> Result<Severity, String> {
    serde_json::from_value(Value::String(s.to_string()))
        .map_err(|_| "one of minor, major, blocker".to_string())
}

#[derive(Subcommand)]
pub enum StageCmd {
    /// Mark the current stage.
    Enter {
        stage: String,
        #[arg(long, value_parser = role)]
        role: Option<Role>,
    },
}

#[derive(Subcommand)]
pub enum GateCmd {
    /// Stop for approval of the listed documents at the next stop.
    Open {
        stage: String,
        #[arg(long, value_parser = role)]
        role: Option<Role>,
        #[arg(long = "doc")]
        docs: Vec<String>,
        #[arg(long = "artifact")]
        artifacts: Vec<String>,
        #[arg(long)]
        next: Option<String>,
        #[arg(long)]
        needs_human: Option<String>,
        #[arg(long = "question")]
        questions: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum TasksCmd {
    /// Re-read the task list.
    Sync {
        #[arg(long)]
        file: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum TaskCmd {
    Start {
        id: String,
    },
    Done {
        id: String,
        #[arg(long)]
        evidence: String,
    },
    Block {
        id: String,
        #[arg(long)]
        reason: String,
    },
}

#[derive(Subcommand)]
pub enum CommitCmd {
    /// Open a commit gate for exactly these files.
    Propose {
        id: String,
        #[arg(long)]
        subject: String,
        /// The full commit message (`git commit -m`); xherd commits with it
        /// after approval, and the user may edit it first.
        #[arg(long)]
        message: Option<String>,
        #[arg(long, value_delimiter = ',', required = true)]
        files: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        tasks: Vec<String>,
        #[arg(long)]
        needs_human: Option<String>,
    },
    /// The approved proposal is committed as SHA.
    Done {
        id: String,
        #[arg(long)]
        sha: String,
    },
    Drop {
        id: String,
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum PrCmd {
    Opened {
        #[arg(long)]
        url: String,
    },
}

#[derive(Subcommand)]
pub enum HandoffCmd {
    Written {
        #[arg(long)]
        file: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum IssueCmd {
    Add {
        #[arg(long)]
        title: String,
        #[arg(long, value_parser = severity)]
        severity: Severity,
        #[arg(long)]
        about: Vec<String>,
        #[arg(long)]
        body: Option<String>,
    },
}

impl From<StageCmd> for XapCmd {
    fn from(c: StageCmd) -> XapCmd {
        let StageCmd::Enter { stage, role } = c;
        XapCmd::StageEnter { stage, role }
    }
}

impl From<GateCmd> for XapCmd {
    fn from(c: GateCmd) -> XapCmd {
        let GateCmd::Open {
            stage,
            role,
            docs,
            artifacts,
            next,
            needs_human,
            questions,
        } = c;
        XapCmd::GateOpen {
            stage,
            role,
            docs,
            artifacts,
            next,
            needs_human,
            questions,
        }
    }
}

impl From<TasksCmd> for XapCmd {
    fn from(c: TasksCmd) -> XapCmd {
        let TasksCmd::Sync { file } = c;
        XapCmd::TasksSync { file }
    }
}

impl From<TaskCmd> for XapCmd {
    fn from(c: TaskCmd) -> XapCmd {
        match c {
            TaskCmd::Start { id } => XapCmd::TaskStart { id },
            TaskCmd::Done { id, evidence } => XapCmd::TaskDone { id, evidence },
            TaskCmd::Block { id, reason } => XapCmd::TaskBlock { id, reason },
        }
    }
}

impl From<CommitCmd> for XapCmd {
    fn from(c: CommitCmd) -> XapCmd {
        match c {
            CommitCmd::Propose {
                id,
                subject,
                message,
                files,
                tasks,
                needs_human,
            } => XapCmd::CommitPropose {
                id,
                subject,
                message,
                files,
                tasks,
                needs_human,
            },
            CommitCmd::Done { id, sha } => XapCmd::CommitDone { id, sha },
            CommitCmd::Drop { id, reason } => XapCmd::CommitDrop { id, reason },
        }
    }
}

impl From<PrCmd> for XapCmd {
    fn from(c: PrCmd) -> XapCmd {
        let PrCmd::Opened { url } = c;
        XapCmd::PrOpened { url }
    }
}

impl From<HandoffCmd> for XapCmd {
    fn from(c: HandoffCmd) -> XapCmd {
        let HandoffCmd::Written { file } = c;
        XapCmd::HandoffWritten { file }
    }
}

impl From<IssueCmd> for XapCmd {
    fn from(c: IssueCmd) -> XapCmd {
        let IssueCmd::Add {
            title,
            severity,
            about,
            body,
        } = c;
        XapCmd::IssueAdd {
            title,
            severity,
            about,
            body,
        }
    }
}

enum Posted {
    /// The daemon answered (`ok` or a refusal).
    Answer(Value),
    /// No token, daemon down, timeout or 5xx.
    Unreachable(String),
}

fn post(call: &XapCall) -> Posted {
    let Some(token) = std::env::var("XHERD_AGENT_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
    else {
        return Posted::Unreachable("no XHERD_AGENT_TOKEN in this environment".into());
    };
    let url = std::env::var("XHERD_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_URL.into());
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_millis(500)))
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .new_agent();
    let res = agent
        .post(format!("{}/api/v1/xap", url.trim_end_matches('/')))
        .header("Authorization", format!("Bearer {token}"))
        .send_json(call);
    match res {
        Ok(r) if r.status().is_server_error() => {
            Posted::Unreachable(format!("daemon error {}", r.status()))
        }
        Ok(r) => {
            let status = r.status();
            match r.into_body().read_json::<Value>() {
                Ok(v) if v.get("ok").is_some() => Posted::Answer(v),
                Ok(v) => Posted::Answer(serde_json::json!({
                    "ok": false,
                    "code": v["code"].as_str().unwrap_or("rejected"),
                    "message": v["error"].as_str().map_or_else(|| status.to_string(), str::to_string),
                })),
                Err(e) => Posted::Unreachable(format!("unreadable answer ({status}): {e}")),
            }
        }
        Err(e) => Posted::Unreachable(format!("daemon not reachable at {url}: {e}")),
    }
}

pub fn run(cmd: XapCmd, json: bool) -> i32 {
    let name = cmd.name();
    let Some(target) = find_agent() else {
        eprintln!("xherd {name}: not running under xherd");
        return 4;
    };
    let report_only = cmd.is_report_only();
    let call = XapCall {
        id: xherd_core::new_id(),
        agent_id: target.agent_id,
        cmd,
        at: xherd_core::now_ms(),
    };
    match post(&call) {
        Posted::Answer(v) => {
            let ok = v["ok"].as_bool() == Some(true);
            if json {
                println!("{v}");
            } else if ok {
                let changed = v["result"]["changed"].as_bool() != Some(false);
                println!("{name}: {}", if changed { "accepted" } else { "no change" });
            } else {
                eprintln!(
                    "xherd {name}: {} ({})",
                    v["message"].as_str().unwrap_or("refused"),
                    v["code"].as_str().unwrap_or("?")
                );
            }
            if ok { 0 } else { 2 }
        }
        Posted::Unreachable(why) if report_only => {
            match xherd_core::queue::append(&Paths::from_env().hook_queue(), &call.into()) {
                Ok(()) => {
                    if json {
                        println!(r#"{{"ok":true,"result":{{"queued":true}}}}"#);
                    } else {
                        println!("{name}: queued ({why})");
                    }
                    0
                }
                Err(e) => {
                    eprintln!("xherd {name}: {why}; cannot queue: {e}");
                    3
                }
            }
        }
        Posted::Unreachable(why) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"ok": false, "code": "unreachable", "message": why})
                );
            }
            eprintln!("xherd {name}: {why}; stop and wait, do not retry in a loop");
            3
        }
    }
}
