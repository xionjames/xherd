use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use xherd_core::paths::Paths;
use xherd_core::{Agent, Status};
use xherd_daemon::support::{SCOPE_HOOKS, SCOPE_XAP, TOKEN_AGENT, mint_token};
use xherd_daemon::*;

async fn setup() -> (tempfile::TempDir, Daemon, Agent) {
    let dir = tempfile::tempdir().unwrap();
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    o.scheduler = None;
    // Never the real Claude Code config: a fixture that trusts the workspace.
    let trust = dir.path().join("claude.json");
    let ws = dir.path().join("ws");
    std::fs::write(
        &trust,
        json!({"projects": {ws.to_string_lossy(): {"hasTrustDialogAccepted": true}}}).to_string(),
    )
    .unwrap();
    let rt = o
        .builtin_plugins
        .iter_mut()
        .find(|m| m.name == "claude-code")
        .unwrap();
    rt.data.get_mut("terminal").unwrap()["trust"]["file"] = trust.to_string_lossy().into();
    let paths = Paths::at(dir.path());
    paths.ensure().unwrap();
    xherd_core::config::Config {
        workspace: Some(ws.to_string_lossy().into_owned()),
        ..Default::default()
    }
    .save(&paths.config())
    .unwrap();
    let d = Daemon::start(paths, o).await.unwrap();
    let mut a = Agent::new("SC-1", "SC-1", 1);
    a.launch = Some(vec!["rt".into()]);
    let a = d.create_agent(a).await.unwrap();
    for _ in 0..100 {
        if d.action("SC-1", UserAction::Get).await.unwrap().status == Status::Running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    (dir, d, a)
}

async fn call(app: axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn get(uri: &str) -> axum::http::request::Builder {
    Request::get(uri).header("host", "127.0.0.1:4740")
}

#[tokio::test]
async fn auth_rules() {
    let (_dir, d, _a) = setup().await;
    let remote = xherd_api::router(d.clone())
        .layer(MockConnectInfo(SocketAddr::from(([192, 168, 1, 5], 5555))));
    let local = xherd_api::router(d.clone())
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));

    let (s, v) = call(
        remote.clone(),
        get("/api/v1/healthz").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!((s, v["ok"].clone()), (StatusCode::OK, json!(true)));

    let (s, _) = call(
        remote.clone(),
        get("/api/v1/agents").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, v) = call(
        local.clone(),
        get("/api/v1/agents").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["agents"][0]["name"], "SC-1");

    // Proxied requests are not local even from loopback.
    let req = get("/api/v1/agents")
        .header("x-forwarded-for", "1.2.3.4")
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(local.clone(), req).await.0, StatusCode::UNAUTHORIZED);

    let token = d.config.user_token.clone().unwrap();
    let req = get("/api/v1/state")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let (s, v) = call(remote.clone(), req).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        v["config"].get("userToken").is_none(),
        "token never sent back"
    );
    let req = get("/api/v1/state")
        .header("authorization", "Bearer nope")
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(remote, req).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn hooks_need_the_agents_own_token() {
    let (_dir, d, a) = setup().await;
    let app =
        xherd_api::router(d.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
    let token = mint_token(
        &d.store,
        TOKEN_AGENT,
        Some(&a.id),
        &[SCOPE_HOOKS, SCOPE_XAP],
        1,
    )
    .await
    .unwrap();
    let other = mint_token(
        &d.store,
        TOKEN_AGENT,
        Some("someone-else"),
        &[SCOPE_HOOKS],
        1,
    )
    .await
    .unwrap();
    let body = json!({"id": "r1", "agentId": a.id, "event": "stop", "payload": {"message": "ok"}, "seq": 1, "at": 0});
    let post = |tok: &str, b: &Value| {
        Request::post("/api/v1/hooks")
            .header("authorization", format!("Bearer {tok}"))
            .header("content-type", "application/json")
            .body(Body::from(b.to_string()))
            .unwrap()
    };

    let (s, v) = call(app.clone(), post(&token, &body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["result"], "applied");
    let (_, v) = call(app.clone(), post(&token, &body)).await;
    assert_eq!(v["result"], "duplicate");
    assert_eq!(
        call(app.clone(), post(&other, &body)).await.0,
        StatusCode::FORBIDDEN
    );
    // The user token is not an agent token.
    let user = d.config.user_token.clone().unwrap();
    assert_eq!(
        call(app.clone(), post(&user, &body)).await.0,
        StatusCode::UNAUTHORIZED
    );
    // Strict bodies: an unknown event is rejected.
    let mut bad = body.clone();
    bad["event"] = json!("explode");
    assert_eq!(
        call(app.clone(), post(&token, &bad)).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        d.action("SC-1", UserAction::Get).await.unwrap().status,
        Status::Idle
    );
}

#[tokio::test]
async fn agent_actions() {
    let (_dir, d, _a) = setup().await;
    let app =
        xherd_api::router(d.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
    let post = |uri: &str, body: Option<Value>| {
        let b = Request::post(uri).header("host", "localhost:4740");
        match body {
            Some(v) => b
                .header("content-type", "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => b.body(Body::empty()).unwrap(),
        }
    };
    let (s, v) = call(
        app.clone(),
        post("/api/v1/agents/SC-1/send", Some(json!({"text": "hi"}))),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = call(app.clone(), post("/api/v1/agents/SC-1/pause", None)).await;
    assert_eq!(
        (s, v["agent"]["status"].clone()),
        (StatusCode::OK, json!("paused"))
    );
    let (s, v) = call(app.clone(), post("/api/v1/agents/SC-1/pause", None)).await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("not_allowed"))
    );
    let (s, _) = call(app.clone(), post("/api/v1/agents/NOPE/pause", None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // Only refusals here: a real install writes to the user's runtime folder.
    let (s, v) = call(
        app.clone(),
        post("/api/v1/install", Some(json!({"runtime": "nope"}))),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::UNPROCESSABLE_ENTITY, json!("unknown_runtime"))
    );
    let (s, v) = call(
        app.clone(),
        get("/api/v1/agents/SC-1").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, d) = call(
        app.clone(),
        get("/api/v1/agents/SC-1/diff").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(
        (s, d["code"].clone()),
        (StatusCode::CONFLICT, json!("no_workspace"))
    );
    assert!(
        v["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "user.pause")
    );
}

#[tokio::test]
async fn xap_endpoint() {
    let (_dir, d, a) = setup().await;
    let app =
        xherd_api::router(d.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
    let token = mint_token(
        &d.store,
        TOKEN_AGENT,
        Some(&a.id),
        &[SCOPE_HOOKS, SCOPE_XAP],
        1,
    )
    .await
    .unwrap();
    let hooks_only = mint_token(&d.store, TOKEN_AGENT, Some(&a.id), &[SCOPE_HOOKS], 1)
        .await
        .unwrap();
    let post = |tok: &str, body: String| {
        Request::post("/api/v1/xap")
            .header("host", "127.0.0.1:4740")
            .header("authorization", format!("Bearer {tok}"))
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap()
    };
    let ask =
        json!({"id": "x1", "agentId": a.id, "cmd": "ask", "args": {"text": "which db?"}, "at": 0});

    let (s, v) = call(app.clone(), post(&hooks_only, ask.to_string())).await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::FORBIDDEN, json!("scope"))
    );

    let mut other = ask.clone();
    other["agentId"] = json!("someone-else");
    let (s, v) = call(app.clone(), post(&token, other.to_string())).await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::FORBIDDEN, json!("wrong_agent"))
    );

    let (s, v) = call(app.clone(), post(&token, "{\"cmd\": 1}".into())).await;
    assert_eq!(
        (s, v["ok"].clone(), v["code"].clone()),
        (StatusCode::BAD_REQUEST, json!(false), json!("usage"))
    );

    let (s, v) = call(app.clone(), post(&token, ask.to_string())).await;
    assert_eq!((s, v["ok"].clone()), (StatusCode::OK, json!(true)), "{v}");
    assert_eq!(v["result"]["gate"]["kind"], "question");

    // The setup agent has no flow: stage calls are refused with 422.
    let stage = json!({"id": "x2", "agentId": a.id, "cmd": "stage.enter", "args": {"stage": "spec"}, "at": 0});
    let (s, v) = call(app.clone(), post(&token, stage.to_string())).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        (v["ok"].clone(), v["code"].clone()),
        (json!(false), json!("no_flow"))
    );
    assert!(v["message"].as_str().unwrap().contains("without a flow"));
}

#[tokio::test]
async fn create_agent_endpoint() {
    let (dir, d, _a) = setup().await;
    let app =
        xherd_api::router(d.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
    let post = |body: Value| {
        Request::post("/api/v1/agents")
            .header("host", "127.0.0.1:4740")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let run = |args: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .status()
                .unwrap()
                .success()
        )
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["commit", "-q", "--allow-empty", "-m", "base"]);
    let repo = repo.to_string_lossy().into_owned();

    let (s, v) = call(
        app.clone(),
        post(json!({"ticketKey": "bad key", "repo": repo})),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::UNPROCESSABLE_ENTITY, json!("bad_key"))
    );
    let (s, _) = call(
        app.clone(),
        post(json!({"ticketKey": "SC-9", "repo": repo, "extra": 1})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, v) = call(
        app.clone(),
        post(json!({"ticketKey": "SC-1", "repo": repo})),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("name_taken"))
    );
    let (s, v) = call(
        app.clone(),
        post(json!({"ticketKey": "sc-9", "repo": repo})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["agent"]["name"], "SC-9");
}
