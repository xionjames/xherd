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
use xherd_daemon::support::{SCOPE_HOOKS, SCOPE_XAP, TOKEN_AGENT, TOKEN_USER, mint_token};
use xherd_daemon::*;

async fn setup() -> (tempfile::TempDir, Daemon, Agent) {
    setup_with(|_| {}).await
}

/// `setup`, with `prep` run on the data root before the daemon starts (to
/// add external plugins).
async fn setup_with(prep: impl FnOnce(&Paths)) -> (tempfile::TempDir, Daemon, Agent) {
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
    prep(&paths);
    let d = Daemon::start(paths, o).await.unwrap();
    let mut a = Agent::new("SC-1", 1);
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
    // Locally an unknown token (another install's) changes nothing; an
    // agent's token never becomes the user's.
    let req = get("/api/v1/agents")
        .header("authorization", "Bearer xh_other_install")
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(local.clone(), req).await.0, StatusCode::OK);
    let agent_t = xherd_daemon::support::mint_token(
        &d.store,
        xherd_daemon::support::TOKEN_AGENT,
        Some("x"),
        &["hooks"],
        1,
    )
    .await
    .unwrap();
    let req = get("/api/v1/agents")
        .header("authorization", format!("Bearer {agent_t}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(local.clone(), req).await.0, StatusCode::UNAUTHORIZED);

    // In the container, the gateway the published port arrives from is local.
    let mut d2 = d.clone();
    d2.local_peers = Arc::new(vec!["172.17.0.1".parse().unwrap()]);
    let gw =
        xherd_api::router(d2).layer(MockConnectInfo(SocketAddr::from(([172, 17, 0, 1], 5555))));
    let req = get("/api/v1/agents").body(Body::empty()).unwrap();
    assert_eq!(call(gw, req).await.0, StatusCode::OK);

    // There, loopback is where the agents are: no rights without a token,
    // not even with an unknown one (R-1203, T2).
    let mut d3 = d.clone();
    d3.sandbox = true;
    d3.local_peers = Arc::new(vec!["172.17.0.1".parse().unwrap()]);
    let inside = xherd_api::router(d3.clone())
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));
    let req = get("/api/v1/agents").body(Body::empty()).unwrap();
    assert_eq!(call(inside.clone(), req).await.0, StatusCode::UNAUTHORIZED);
    let req = get("/api/v1/agents")
        .header("authorization", "Bearer never-issued")
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(inside.clone(), req).await.0, StatusCode::UNAUTHORIZED);
    // The user's own token still works from inside (the CLI).
    let user_t = mint_token(&d3.store, TOKEN_USER, None, &[], 1)
        .await
        .unwrap();
    let req = get("/api/v1/agents")
        .header("authorization", format!("Bearer {user_t}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(inside, req).await.0, StatusCode::OK);
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

    for (body, code) in [
        (json!({"name": "Bad Name", "repo": repo}), "bad_name"),
        (
            json!({"name": "x", "ticket": "bad key", "repo": repo}),
            "bad_ticket",
        ),
        (json!({"repo": repo}), "name_required"),
    ] {
        let (s, v) = call(app.clone(), post(body)).await;
        assert_eq!(
            (s, v["code"].clone()),
            (StatusCode::UNPROCESSABLE_ENTITY, json!(code))
        );
    }
    let (s, _) = call(
        app.clone(),
        post(json!({"name": "sc-9", "repo": repo, "extra": 1})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, v) = call(
        app.clone(),
        post(json!({"name": "refund-retry", "ticket": "sc-9", "repo": repo})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(
        (
            &v["agent"]["name"],
            &v["agent"]["ticket"],
            &v["agent"]["branch"]
        ),
        (
            &json!("refund-retry"),
            &json!({"key": "SC-9"}),
            &json!("SC-9-refund-retry")
        )
    );
    let (s, v) = call(
        app.clone(),
        post(json!({"name": "refund-retry", "repo": repo})),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("name_taken"))
    );
    // The fields from before names (#114) still work for one release.
    let (s, v) = call(
        app.clone(),
        post(json!({"ticketKey": "SC-10", "repo": repo})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(
        (&v["agent"]["name"], &v["agent"]["branch"]),
        (&json!("sc-10"), &json!("SC-10"))
    );
    // A ticket URL no tracker reads is kept as a link, with no key.
    let (s, v) = call(
        app.clone(),
        post(json!({"name": "linked", "ticket": "https://example.org/t/1", "repo": repo})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(
        (&v["agent"]["ticket"], &v["agent"]["branch"]),
        (&json!({"url": "https://example.org/t/1"}), &json!("linked"))
    );
}

#[tokio::test]
async fn cross_site_origin_is_refused() {
    let (_dir, d, _a) = setup().await;
    let local = xherd_api::router(d.clone())
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));
    let (s, v) = call(
        local.clone(),
        get("/api/v1/state")
            .header("origin", "http://evil.example")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    let (s, v) = call(
        local,
        get("/api/v1/state")
            .header("origin", "http://localhost:5173")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let feature = v["flows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["flow"] == "feature")
        .unwrap();
    assert_eq!(feature["stages"][0]["id"], "spec");
    assert_eq!(feature["stages"][0]["gate"], true);
}

#[tokio::test]
async fn serves_the_ui_page() {
    let (_dir, d, _a) = setup().await;
    let local =
        xherd_api::router(d).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));
    let res = local
        .oneshot(get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res.headers()["content-type"].to_str().unwrap().to_string();
    assert!(ct.starts_with("text/html"), "{ct}");
}

#[tokio::test]
async fn websocket_streams_agent_changes() {
    use futures_util::StreamExt;
    let (_dir, d, _a) = setup().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(xherd_api::serve(
        d.clone(),
        listener,
        std::future::pending(),
    ));
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/v1/ws"))
        .await
        .unwrap();
    d.action("SC-1", UserAction::Pause).await.unwrap();
    let msg = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let m = ws.next().await.unwrap().unwrap();
            let v: Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
            if v["type"] == "agent" && v["agent"]["status"] == "paused" {
                return v;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(msg["agent"]["name"], "SC-1");
}

#[tokio::test]
async fn terminal_websocket_attaches_to_the_session() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let (_dir, d, _a) = setup().await;
    // The fake driver starts no tmux session: make one on a private socket.
    let sock = format!("xherd-test-term-{}", std::process::id());
    let tmux = |args: &[&str]| {
        std::process::Command::new("tmux")
            .args(["-L", &sock])
            .args(args)
            .status()
            .unwrap()
    };
    assert!(
        tmux(&[
            "new-session",
            "-d",
            "-s",
            "SC-1",
            "-x",
            "80",
            "-y",
            "24",
            "cat"
        ])
        .success()
    );
    let mut cfg = (*d.config).clone();
    cfg.tmux_socket = Some(sock.clone());
    let d = Daemon {
        config: Arc::new(cfg),
        ..d
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(xherd_api::serve(
        d.clone(),
        listener,
        std::future::pending(),
    ));

    let url = format!("ws://127.0.0.1:{port}/api/v1/ws/term/SC-1?cols=80&rows=24");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws.send(Message::text(r#"{"type":"resize","cols":90,"rows":25}"#))
        .await
        .unwrap();
    ws.send(Message::binary(b"hello-term\r".to_vec()))
        .await
        .unwrap();
    let mut seen = String::new();
    let found = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(m)) = ws.next().await {
            if let Message::Binary(b) = m {
                seen.push_str(&String::from_utf8_lossy(&b));
                if seen.contains("hello-term") {
                    return true;
                }
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    drop(ws);
    // Detaching leaves the session running.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let alive = tmux(&["has-session", "-t", "=SC-1"]).success();
    tmux(&["kill-server"]);
    assert!(found, "no echo in {seen:?}");
    assert!(alive);
}

/// Settings → Git access and `xherd credential` (R-684, R-685): the token is
/// stored but never sent back; the credential endpoint answers the local
/// user or an agent token with the `credential` scope only.
#[tokio::test]
async fn git_access_and_credentials() {
    let (dir, d, a) = setup().await;
    let local =
        xherd_api::router(d.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
    let remote =
        xherd_api::router(d.clone()).layer(MockConnectInfo(SocketAddr::from(([10, 0, 0, 9], 1))));
    let body = json!({
        "hosts": [{"host": "git.example.test", "credential": "xherd", "provider": "gitlab", "protocols": ["https"]}],
        "tokens": {"git.example.test": "tok_SECRET_9"},
    });
    let req = Request::put("/api/v1/git/hosts")
        .header("host", "127.0.0.1:4740")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let (s, v) = call(local.clone(), req).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["hosts"][0]["hasToken"], true);
    assert!(
        !v.to_string().contains("tok_SECRET_9"),
        "token never sent back"
    );
    // The actors see the provider at once (they pick a PR host by it).
    let live = d.settings.get();
    assert_eq!(live.git_hosts[0].provider.as_deref(), Some("gitlab"));
    // Natively the managed config sits in the data root, never in $HOME.
    let gc = std::fs::read_to_string(dir.path().join("gitconfig")).unwrap();
    assert!(gc.contains("[credential \"https://git.example.test\"]"));

    let cred = |host: &str, auth: Option<&str>| {
        let mut r = Request::post("/api/v1/git/credential")
            .header("host", "127.0.0.1:4740")
            .header("content-type", "application/json");
        if let Some(t) = auth {
            r = r.header("authorization", format!("Bearer {t}"));
        }
        r.body(Body::from(
            json!({"protocol": "https", "host": host}).to_string(),
        ))
        .unwrap()
    };
    let (s, v) = call(local.clone(), cred("git.example.test", None)).await;
    assert_eq!(
        (s, v["password"].clone()),
        (StatusCode::OK, json!("tok_SECRET_9"))
    );
    assert_eq!(
        call(local, cred("other.test", None)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(remote.clone(), cred("git.example.test", None)).await.0,
        StatusCode::UNAUTHORIZED
    );
    // An agent's token carries the credential scope.
    let t = xherd_daemon::support::mint_token(
        &d.store,
        xherd_daemon::support::TOKEN_AGENT,
        Some(&a.id),
        &[xherd_daemon::SCOPE_CREDENTIAL],
        1,
    )
    .await
    .unwrap();
    assert_eq!(
        call(remote.clone(), cred("git.example.test", Some(&t)))
            .await
            .0,
        StatusCode::OK
    );
    let hooks_only = xherd_daemon::support::mint_token(
        &d.store,
        xherd_daemon::support::TOKEN_AGENT,
        Some(&a.id),
        &["hooks"],
        1,
    )
    .await
    .unwrap();
    assert_eq!(
        call(remote, cred("git.example.test", Some(&hooks_only)))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn security_overview_needs_the_user_and_lists_findings() {
    let (_dir, d, _a) = setup().await;
    let remote = xherd_api::router(d.clone())
        .layer(MockConnectInfo(SocketAddr::from(([192, 168, 1, 5], 5555))));
    let local =
        xherd_api::router(d).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));
    let req = || get("/api/v1/security").body(Body::empty()).unwrap();
    assert_eq!(call(remote, req()).await.0, StatusCode::UNAUTHORIZED);
    let (s, v) = call(local, req()).await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"network") && ids.contains(&"session-limit"));
    assert!(!v["pending"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn findings_endpoint_reads_review_md() {
    let (dir, d, _a) = setup().await;
    let wt = dir.path().join("wt");
    let spec = wt.join(".xherd/spec");
    std::fs::create_dir_all(&spec).unwrap();
    let mut b = Agent::new("SC-2", 2);
    b.launch = Some(vec!["rt".into()]);
    b.workdir = Some(wt.to_string_lossy().into_owned());
    d.create_agent(b).await.unwrap();
    let local =
        xherd_api::router(d).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));
    let req = || {
        get("/api/v1/agents/SC-2/findings")
            .body(Body::empty())
            .unwrap()
    };
    let (s, v) = call(local.clone(), req()).await;
    assert_eq!(
        (s, v["findings"].as_array().map(Vec::len)),
        (StatusCode::OK, Some(0))
    );

    std::fs::write(
        spec.join("review.md"),
        "### F01: race\n- severity: blocker\n- location: `a.rs:3`\n",
    )
    .unwrap();
    let (_, v) = call(local.clone(), req()).await;
    let f = &v["findings"][0];
    assert_eq!(f["id"], "F01");
    assert_eq!(f["severity"], "blocker");
    assert_eq!(f["location"], "a.rs:3");
    assert_eq!(f["resolved"], false);

    std::fs::write(spec.join("review.md"), "### F01: race\n").unwrap();
    let (_, v) = call(local, req()).await;
    assert!(
        v["error"].as_str().unwrap().contains("no `- severity"),
        "{v}"
    );
}

/// A plugin's secret settings are set from the UI: the value is stored in
/// secrets.json and never sent back; only declared secrets are accepted.
#[tokio::test]
async fn plugin_secrets_are_write_only() {
    let (dir, d, _a) = setup().await;
    let local =
        xherd_api::router(d.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
    let req = |method: &str, body: Option<serde_json::Value>| {
        Request::builder()
            .method(method)
            .uri("/api/v1/plugins/github/secrets")
            .header("host", "127.0.0.1:4740")
            .header("content-type", "application/json")
            .body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
            .unwrap()
    };
    let (s, v) = call(local.clone(), req("GET", None)).await;
    assert_eq!((s, v["token"].as_bool()), (StatusCode::OK, Some(false)));

    let (s, v) = call(
        local.clone(),
        req("PUT", Some(json!({"values": {"token": "ghp_SECRET_77"}}))),
    )
    .await;
    assert_eq!((s, v["token"].as_bool()), (StatusCode::OK, Some(true)));
    assert!(!v.to_string().contains("ghp_SECRET_77"));
    let file = std::fs::read_to_string(dir.path().join("secrets.json")).unwrap();
    assert!(file.contains("ghp_SECRET_77"));

    // An undeclared key and an unknown plugin are refused.
    let (s, _) = call(
        local.clone(),
        req("PUT", Some(json!({"values": {"nope": "x"}}))),
    )
    .await;
    assert_ne!(s, StatusCode::OK);

    let (_, v) = call(local, req("PUT", Some(json!({"values": {"token": null}})))).await;
    assert_eq!(v["token"].as_bool(), Some(false));
    let file = std::fs::read_to_string(dir.path().join("secrets.json")).unwrap();
    assert!(!file.contains("ghp_SECRET_77"));
}

async fn fetch(app: axum::Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let res = app
        .oneshot(get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (parts, body) = res.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes().to_vec();
    (parts.status, parts.headers, bytes)
}

/// An external view plugin `my-view` (with a file over the size limit, a
/// hidden file and a symlink out of its folder) and a non-view plugin
/// `my-host` with an `index.html`.
fn external_views(paths: &Paths) {
    let dir = paths.plugins().join("my-view");
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(
        dir.join("xherd-plugin.json"),
        r#"{"name": "my-view", "kind": "view", "version": "0.1.0", "label": "Mine",
            "data": {"view": {"entry": "index.html"}}}"#,
    )
    .unwrap();
    std::fs::write(dir.join("index.html"), "<p>mine</p>").unwrap();
    std::fs::write(dir.join("assets/app.js"), "void 0;").unwrap();
    std::fs::write(dir.join(".env"), "SECRET=1").unwrap();
    std::fs::File::create(dir.join("big.js"))
        .unwrap()
        .set_len(xherd_plugin::view::MAX_FILE + 1)
        .unwrap();
    let outside = paths.root.join("outside.js");
    std::fs::write(&outside, "outside").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("link.js")).unwrap();
    std::os::unix::fs::symlink(".env", dir.join("notes.txt")).unwrap();
    let host = paths.plugins().join("my-host");
    std::fs::create_dir_all(&host).unwrap();
    std::fs::write(
        host.join("xherd-plugin.json"),
        r#"{"name": "my-host", "kind": "host", "version": "0.1.0"}"#,
    )
    .unwrap();
    std::fs::write(host.join("index.html"), "<p>host</p>").unwrap();
}

/// R8, R10: view files are served to anyone, with the locks of design §3:
/// CSP `sandbox allow-scripts` and path-restricted sources, `nosniff`,
/// `no-referrer`, and CORS open (the frame's origin is opaque).
#[tokio::test]
async fn view_plugin_files_are_served_sandboxed() {
    let (_dir, d, _a) = setup_with(external_views).await;
    let remote = xherd_api::router(d.clone())
        .layer(MockConnectInfo(SocketAddr::from(([192, 168, 1, 5], 5555))));

    let (s, h, body) = fetch(remote.clone(), "/api/v1/plugins/herd-tiles/view/index.html").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["content-type"], "text/html; charset=utf-8");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["referrer-policy"], "no-referrer");
    assert_eq!(h["access-control-allow-origin"], "*");
    assert_eq!(h["cache-control"], "no-cache");
    let csp = h["content-security-policy"].to_str().unwrap();
    assert!(csp.starts_with("sandbox allow-scripts;"), "{csp}");
    assert!(csp.contains("connect-src 'none'"), "{csp}");
    assert!(
        csp.contains("script-src http://127.0.0.1:4740/api/v1/plugins/herd-tiles/view/ "),
        "{csp}"
    );
    assert_eq!(
        body,
        xherd_plugin::view::builtin_file("herd-tiles", "index.html").unwrap()
    );
    let (s, h, _) = fetch(remote.clone(), "/api/v1/plugins/herd-tiles/view/tiles.js").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["content-type"], "text/javascript; charset=utf-8");

    let (s, h, body) = fetch(remote.clone(), "/api/v1/plugins/my-view/view/index.html").await;
    assert_eq!((s, body.as_slice()), (StatusCode::OK, &b"<p>mine</p>"[..]));
    assert!(
        h["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("/api/v1/plugins/my-view/view/")
    );
    let (s, _, body) = fetch(remote.clone(), "/api/v1/plugins/my-view/view/assets/app.js").await;
    assert_eq!((s, body.as_slice()), (StatusCode::OK, &b"void 0;"[..]));

    for uri in [
        "/api/v1/plugins/my-view/view/../my-host/index.html",
        "/api/v1/plugins/my-view/view/assets/../index.html",
        "/api/v1/plugins/my-view/view/%2e%2e/my-host/index.html",
        "/api/v1/plugins/my-view/view/%2E%2E%2Foutside.js",
        "/api/v1/plugins/my-view/view/.env",
        "/api/v1/plugins/my-view/view/link.js",
        "/api/v1/plugins/my-view/view/notes.txt",
        "/api/v1/plugins/my-view/view/missing.js",
        "/api/v1/plugins/my-view/view/assets",
        "/api/v1/plugins/my-host/view/index.html",
        "/api/v1/plugins/github/view/index.html",
        "/api/v1/plugins/nope/view/index.html",
        "/api/v1/plugins/herd-tiles/view/missing.js",
    ] {
        assert_eq!(
            fetch(remote.clone(), uri).await.0,
            StatusCode::NOT_FOUND,
            "{uri}"
        );
    }
    assert_eq!(
        fetch(remote.clone(), "/api/v1/plugins/my-view/view/big.js")
            .await
            .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );

    // A view plugin that is off is not served.
    let patch =
        serde_json::from_value(json!({"disabledPlugins": ["my-view", "herd-tiles"]})).unwrap();
    d.save_config(patch).unwrap();
    for uri in [
        "/api/v1/plugins/my-view/view/index.html",
        "/api/v1/plugins/herd-tiles/view/index.html",
    ] {
        assert_eq!(
            fetch(remote.clone(), uri).await.0,
            StatusCode::NOT_FOUND,
            "{uri}"
        );
    }
}

/// R10: the view kit (xherd's stylesheet and the protocol script) is public
/// and locked like view files.
#[tokio::test]
async fn view_kit_is_served() {
    let (_dir, d, _a) = setup().await;
    let remote =
        xherd_api::router(d).layer(MockConnectInfo(SocketAddr::from(([192, 168, 1, 5], 1))));
    let (s, h, body) = fetch(remote.clone(), "/api/v1/view-kit/xherd.css").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["content-type"], "text/css; charset=utf-8");
    assert_eq!(h["access-control-allow-origin"], "*");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert!(String::from_utf8(body).unwrap().contains("--accent"));
    let (s, h, body) = fetch(remote.clone(), "/api/v1/view-kit/xherd-view.js").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["content-type"], "text/javascript; charset=utf-8");
    assert!(
        h["content-security-policy"]
            .to_str()
            .unwrap()
            .starts_with("sandbox")
    );
    assert!(String::from_utf8(body).unwrap().contains("xherdView"));
    assert_eq!(
        fetch(remote, "/api/v1/view-kit/app.css").await.0,
        StatusCode::NOT_FOUND
    );
}

/// R11: a view runs with an opaque origin (`Origin: null`): it never gets the
/// local user's rights, not for the state nor for the live stream; `/state`
/// tells the UI where a view starts.
#[tokio::test]
async fn opaque_origin_gets_no_rights() {
    let (_dir, d, _a) = setup_with(external_views).await;
    let local = xherd_api::router(d.clone())
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));
    for uri in ["/api/v1/state", "/api/v1/agents"] {
        let req = get(uri)
            .header("origin", "null")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            call(local.clone(), req).await.0,
            StatusCode::UNAUTHORIZED,
            "{uri}"
        );
    }
    let req = get("/api/v1/ws")
        .header("origin", "null")
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(local.clone(), req).await.0, StatusCode::UNAUTHORIZED);

    let (s, v) = call(local, get("/api/v1/state").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::OK);
    let plugin = |name: &str| {
        v["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .unwrap()
            .clone()
    };
    assert_eq!(plugin("herd-tiles")["view"], json!({"entry": "index.html"}));
    assert_eq!(plugin("my-view")["view"], json!({"entry": "index.html"}));
    assert!(plugin("github").get("view").is_none());
}
