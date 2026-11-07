//! `xherd credential` (git credential helper, R-684) and `xherd git doctor`
//! (R-687). Both ask the daemon, which holds the tokens and runs the checks
//! where the agents run.

use std::io::Read;
use std::time::Duration;

use serde_json::{Value, json};
use xherd_core::config::Config;
use xherd_core::paths::Paths;

/// `key=value` lines up to the first empty line (git's helper protocol).
pub fn parse_request(text: &str) -> Vec<(String, String)> {
    text.lines()
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// git calls `xherd credential get|store|erase`. Only `get` answers; any
/// failure prints nothing, so git asks its next helper (never an error).
pub fn credential(op: &str) -> i32 {
    if op != "get" {
        return 0;
    }
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return 0;
    }
    let req = parse_request(&input);
    let get = |k: &str| req.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let (Some(protocol), Some(host)) = (get("protocol"), get("host")) else {
        return 0;
    };
    let cfg = Config::load(&Paths::from_env().config())
        .map(|(c, _)| c)
        .unwrap_or_default();
    let url = std::env::var("XHERD_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| cfg.url());
    let token = std::env::var("XHERD_AGENT_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .or(cfg.user_token);
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .new_agent();
    let mut r = agent.post(format!(
        "{}/api/v1/git/credential",
        url.trim_end_matches('/')
    ));
    if let Some(t) = token {
        r = r.header("Authorization", format!("Bearer {t}"));
    }
    let Ok(resp) = r.send_json(json!({"protocol": protocol, "host": host})) else {
        return 0;
    };
    if !resp.status().is_success() {
        return 0;
    }
    let v: Value = resp.into_body().read_json().unwrap_or(Value::Null);
    if let (Some(u), Some(p)) = (v["username"].as_str(), v["password"].as_str()) {
        println!("username={u}\npassword={p}");
    }
    0
}

/// Prints each finding in order; the first failure gets its fix.
pub fn doctor(client: &crate::Client, host: Option<String>, repo: Option<String>) -> i32 {
    let host = host
        .or_else(|| {
            repo.as_deref()
                .and_then(xherd_daemon::git_access::parse_remote)
                .map(|(h, _)| h)
        })
        .unwrap_or_else(|| "github.com".into());
    let v = match client.call(
        "POST",
        "/git/doctor",
        Some(json!({"host": host, "repo": repo})),
    ) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("xherd git doctor: {e}");
            return 2;
        }
    };
    let mut failed = false;
    for f in v["findings"].as_array().into_iter().flatten() {
        let ok = f["ok"].as_bool().unwrap_or(false);
        println!(
            "{} {:<44} {}",
            if ok { "ok  " } else { "FAIL" },
            f["label"].as_str().unwrap_or("?"),
            f["detail"].as_str().unwrap_or("")
        );
        if !ok {
            failed = true;
            match f["fix"]["kind"].as_str() {
                Some("command") => {
                    println!("     fix: {}", f["fix"]["text"].as_str().unwrap_or(""))
                }
                Some(_) => println!(
                    "     fix: {} (Settings → Git access)",
                    f["fix"]["label"].as_str().unwrap_or("")
                ),
                None => {}
            }
        }
    }
    i32::from(failed)
}

#[cfg(test)]
mod tests {
    #[test]
    fn helper_protocol() {
        let r =
            super::parse_request("protocol=https\nhost=github.com\npath=a/b.git\n\nignored=1\n");
        assert_eq!(
            r,
            [
                ("protocol".to_string(), "https".to_string()),
                ("host".into(), "github.com".into()),
                ("path".into(), "a/b.git".into())
            ]
        );
    }
}
