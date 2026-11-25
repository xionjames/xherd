//! `xherd hook` and `xherd report`: the status hot path (R-012..R-014, R-901).
//! Synchronous, no async runtime. Reports go to the daemon with the agent
//! token; when it cannot be reached they are appended to `hook-queue.jsonl`.

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use xherd_core::paths::Paths;
use xherd_core::report::{Report, ReportEvent};

pub const DEFAULT_URL: &str = "http://127.0.0.1:4740";
const STDIN_LIMIT: u64 = 1024 * 1024;

/// The files that name a job in `.xherd/`, new name first.
const JOB_FILES: [&str; 2] = ["job.json", "agent.json"];

pub struct Target {
    pub job_id: String,
    /// The agent's `.xherd/` folder, when found (sequence counter lives there).
    pub xherd_dir: Option<PathBuf>,
}

/// The first non-empty of the job variable and the one older sessions have
/// (`XHERD_JOB_*`, else `XHERD_AGENT_*`; #141).
pub fn job_env(name: &str) -> Option<String> {
    ["JOB", "AGENT"]
        .iter()
        .filter_map(|k| std::env::var(format!("XHERD_{k}_{name}")).ok())
        .find(|s| !s.is_empty())
}

/// `XHERD_JOB_ID` (else `XHERD_AGENT_ID`), else `.xherd/job.json` (else
/// `agent.json`) in the current directory or a parent.
pub fn find_job() -> Option<Target> {
    let cwd = std::env::current_dir().ok();
    let found = cwd.as_deref().and_then(|d| {
        d.ancestors()
            .map(|a| a.join(".xherd"))
            .find(|x| JOB_FILES.iter().any(|f| x.join(f).is_file()))
    });
    let from_file = found.as_ref().and_then(|x| {
        let file = JOB_FILES.iter().map(|f| x.join(f)).find(|p| p.is_file())?;
        let v: Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
        v.get("id")?.as_str().map(str::to_string)
    });
    let job_id = job_env("ID").or(from_file)?;
    Some(Target {
        job_id,
        xherd_dir: found,
    })
}

fn now_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Monotonic per agent: `max(last + 1, now in µs)`, persisted in `.xherd/seq`
/// under a lock. Without a `.xherd/` folder, the current time in µs.
pub fn next_seq(xherd_dir: Option<&Path>) -> u64 {
    let now = now_micros();
    let Some(dir) = xherd_dir else { return now };
    let inner = || -> std::io::Result<u64> {
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("seq"))?;
        let mut lock = fd_lock::RwLock::new(f);
        let mut g = lock.write()?;
        let mut s = String::new();
        g.read_to_string(&mut s)?;
        let next = s
            .trim()
            .parse::<u64>()
            .map_or(now, |last| (last + 1).max(now));
        use std::io::Seek;
        g.set_len(0)?;
        g.seek(std::io::SeekFrom::Start(0))?;
        g.write_all(next.to_string().as_bytes())?;
        Ok(next)
    };
    inner().unwrap_or(now)
}

/// What Claude Code gets on stdout for the daemon's answer; `None` (nothing
/// printed, Claude Code asks in the terminal) for no answer, an unknown one,
/// or `allow-always` when the request had no suggestions to pass back (R18).
fn permission_output(answer: Option<&str>, suggestions: Option<&Value>) -> Option<Value> {
    let decision = match answer? {
        "allow" => json!({"behavior": "allow"}),
        "allow-always" => json!({"behavior": "allow", "updatedPermissions": suggestions?}),
        "deny" => json!({"behavior": "deny", "message": "Denied in xherd."}),
        _ => return None,
    };
    Some(json!({"hookSpecificOutput": {
        "hookEventName": "PermissionRequest",
        "decision": decision,
    }}))
}

/// `xherd permission-request`: the runtime's blocking permission hook (R18).
/// Waits for the user's answer through the daemon and prints Claude Code's
/// decision. Prints nothing, never queues and never fails when there is no
/// token, no answer, an error or a timeout, so Claude Code asks in the
/// terminal: never a silent allow.
pub fn permission_request() {
    let mut raw = String::new();
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        let _ = stdin.lock().take(STDIN_LIMIT).read_to_string(&mut raw);
    }
    let (Some(target), Some(token)) = (find_job(), job_env("TOKEN")) else {
        return;
    };
    let Ok(input) = serde_json::from_str::<Value>(&raw) else {
        return;
    };
    let suggestions = input
        .get("permission_suggestions")
        .filter(|s| s.as_array().is_some_and(|a| !a.is_empty()));
    let text = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    let body = json!({
        "jobId": target.job_id,
        "sessionId": text("session_id"),
        "toolName": text("tool_name"),
        "toolInput": input.get("tool_input").cloned().unwrap_or(Value::Null),
        "suggestions": suggestions.is_some(),
    });
    let url = std::env::var("XHERD_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_URL.into());
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_millis(300)))
        .timeout_global(Some(Duration::from_secs(3630)))
        .build()
        .new_agent();
    let Ok(resp) = agent
        .post(format!(
            "{}/api/v1/hooks/permission",
            url.trim_end_matches('/')
        ))
        .header("Authorization", format!("Bearer {token}"))
        .send_json(&body)
    else {
        return;
    };
    if !resp.status().is_success() {
        return;
    }
    let Ok(answer) = resp.into_body().read_json::<Value>() else {
        return;
    };
    if let Some(out) = permission_output(answer.get("answer").and_then(Value::as_str), suggestions)
    {
        let _ = writeln!(std::io::stdout(), "{out}");
    }
}

pub enum Delivered {
    Sent,
    Queued,
    /// The daemon answered with an error that a retry would not fix.
    Rejected(String),
}

/// POST to the daemon; queue on connection failure, timeout or 5xx.
pub fn deliver(r: &Report) -> Delivered {
    let url = std::env::var("XHERD_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_URL.into());
    if let Some(token) = job_env("TOKEN") {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_millis(300)))
            .timeout_global(Some(Duration::from_secs(3)))
            .build()
            .new_agent();
        let res = agent
            .post(format!("{}/api/v1/hooks", url.trim_end_matches('/')))
            .header("Authorization", format!("Bearer {token}"))
            .send_json(r);
        match res {
            Ok(resp) if resp.status().is_success() => return Delivered::Sent,
            Ok(resp) if resp.status().is_client_error() => {
                let status = resp.status();
                let body = resp.into_body().read_to_string().unwrap_or_default();
                return Delivered::Rejected(format!("{status}: {body}"));
            }
            _ => {}
        }
    }
    // No token (e.g. a wrapper outside the session) or daemon unreachable.
    match xherd_core::queue::append(&Paths::from_env().hook_queue(), &r.clone().into()) {
        Ok(()) => Delivered::Queued,
        Err(e) => Delivered::Rejected(format!("cannot queue report: {e}")),
    }
}

fn build(t: &Target, event: ReportEvent, payload: Value, seq: Option<u64>) -> Report {
    Report {
        id: xherd_core::new_id(),
        job_id: t.job_id.clone(),
        event,
        payload,
        seq: seq.unwrap_or_else(|| next_seq(t.xherd_dir.as_deref())),
        at: xherd_core::now_ms(),
    }
}

/// `xherd hook <event>`: never prints to stdout, never fails (R-012).
pub fn hook(event: &str) {
    let Some(event) = ReportEvent::parse(event).filter(|e| {
        matches!(
            e,
            ReportEvent::SessionStart
                | ReportEvent::UserPromptSubmit
                | ReportEvent::PostToolUse
                | ReportEvent::Notification
                | ReportEvent::Stop
        )
    }) else {
        return;
    };
    let mut raw = String::new();
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        let _ = stdin.lock().take(STDIN_LIMIT).read_to_string(&mut raw);
    }
    let Some(target) = find_job() else { return };
    let payload = match serde_json::from_str::<Value>(&raw) {
        Ok(v) => v,
        Err(_) if raw.trim().is_empty() => Value::Null,
        Err(_) => json!({"raw": raw}),
    };
    let _ = deliver(&build(&target, event, payload, None));
}

pub struct ReportArgs {
    pub state: Option<String>,
    pub release: bool,
    pub message: Option<String>,
    pub session: Option<String>,
    pub resume_cmd: Option<String>,
    pub seq: Option<u64>,
}

/// `xherd report`: exit 0 (sent or queued), 4 (not under xherd), 64 (usage).
pub fn report(a: ReportArgs) -> i32 {
    let event = match (a.release, a.state.as_deref()) {
        (true, None) => ReportEvent::Release,
        (false, Some("working")) => ReportEvent::Working,
        (false, Some("idle")) => ReportEvent::Idle,
        (false, Some("blocked")) => ReportEvent::Blocked,
        (true, Some(_)) => {
            eprintln!("xherd report: use either --state or --release");
            return 64;
        }
        _ => {
            eprintln!("xherd report: --state working|idle|blocked or --release is required");
            return 64;
        }
    };
    let under_xherd = std::env::var("XHERD_ENV").is_ok_and(|v| v == "1");
    let Some(target) = find_job() else {
        if !under_xherd {
            eprintln!("xherd report: not running under xherd");
        } else {
            eprintln!("xherd report: XHERD_ENV=1 but no XHERD_JOB_ID or .xherd/job.json");
        }
        return 4;
    };
    let mut payload = serde_json::Map::new();
    for (k, v) in [
        ("message", a.message),
        ("session", a.session),
        ("resumeCmd", a.resume_cmd),
    ] {
        if let Some(v) = v {
            payload.insert(k.into(), v.into());
        }
    }
    let payload = if payload.is_empty() {
        Value::Null
    } else {
        Value::Object(payload)
    };
    match deliver(&build(&target, event, payload, a.seq)) {
        Delivered::Sent | Delivered::Queued => 0,
        Delivered::Rejected(why) => {
            eprintln!("xherd report: {why}");
            0
        }
    }
}
