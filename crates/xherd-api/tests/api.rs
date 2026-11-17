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
use xherd_core::{Job, Status};
use xherd_daemon::support::{SCOPE_HOOKS, SCOPE_XAP, TOKEN_AGENT, TOKEN_USER, mint_token};
use xherd_daemon::*;

async fn setup() -> (tempfile::TempDir, Daemon, Job) {
    setup_with(|_| {}).await
}

/// `setup`, with `prep` run on the data root before the daemon starts (to
/// add external plugins).
async fn setup_with(prep: impl FnOnce(&Paths)) -> (tempfile::TempDir, Daemon, Job) {
    setup_opts(|_| {}, prep).await
}

async fn setup_opts(
    tweak: impl FnOnce(&mut Options),
    prep: impl FnOnce(&Paths),
) -> (tempfile::TempDir, Daemon, Job) {
    let dir = tempfile::tempdir().unwrap();
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    o.scheduler = None;
    tweak(&mut o);
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
    let mut a = Job::new("SC-1", 1);
    a.launch = Some(vec!["rt".into()]);
    let a = d.create_job(a).await.unwrap();
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
        get("/api/v1/jobs").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, v) = call(
        local.clone(),
        get("/api/v1/jobs").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["jobs"][0]["name"], "SC-1");

    // Proxied requests are not local even from loopback.
    let req = get("/api/v1/jobs")
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
    let req = get("/api/v1/jobs")
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
    let req = get("/api/v1/jobs")
        .header("authorization", format!("Bearer {agent_t}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(local.clone(), req).await.0, StatusCode::UNAUTHORIZED);

    // In the container, the gateway the published port arrives from is local.
    let mut d2 = d.clone();
    d2.local_peers = Arc::new(vec!["172.17.0.1".parse().unwrap()]);
    let gw =
        xherd_api::router(d2).layer(MockConnectInfo(SocketAddr::from(([172, 17, 0, 1], 5555))));
    let req = get("/api/v1/jobs").body(Body::empty()).unwrap();
    assert_eq!(call(gw, req).await.0, StatusCode::OK);

    // There, loopback is where the agents are: no rights without a token,
    // not even with an unknown one (R-1203, T2).
    let mut d3 = d.clone();
    d3.sandbox = true;
    d3.local_peers = Arc::new(vec!["172.17.0.1".parse().unwrap()]);
    let inside = xherd_api::router(d3.clone())
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));
    let req = get("/api/v1/jobs").body(Body::empty()).unwrap();
    assert_eq!(call(inside.clone(), req).await.0, StatusCode::UNAUTHORIZED);
    let req = get("/api/v1/jobs")
        .header("authorization", "Bearer never-issued")
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(inside.clone(), req).await.0, StatusCode::UNAUTHORIZED);
    // The user's own token still works from inside (the CLI).
    let user_t = mint_token(&d3.store, TOKEN_USER, None, &[], 1)
        .await
        .unwrap();
    let req = get("/api/v1/jobs")
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
    let body = json!({"id": "r1", "jobId": a.id, "event": "stop", "payload": {"message": "ok"}, "seq": 1, "at": 0});
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
        post("/api/v1/jobs/SC-1/send", Some(json!({"text": "hi"}))),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = call(app.clone(), post("/api/v1/jobs/SC-1/pause", None)).await;
    assert_eq!(
        (s, v["job"]["status"].clone()),
        (StatusCode::OK, json!("paused"))
    );
    let (s, v) = call(app.clone(), post("/api/v1/jobs/SC-1/pause", None)).await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("not_allowed"))
    );
    let (s, _) = call(app.clone(), post("/api/v1/jobs/NOPE/pause", None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // Check now (issue #131): refused without a pull request URL.
    let (s, v) = call(app.clone(), post("/api/v1/jobs/SC-1/pr-check", None)).await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("no_pr"))
    );
    // Resolve conflicts (issue #139): refused without a pull request URL too.
    let (s, v) = call(
        app.clone(),
        post("/api/v1/jobs/SC-1/resolve-conflicts", None),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("no_pr"))
    );
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
        get("/api/v1/jobs/SC-1").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, d) = call(
        app.clone(),
        get("/api/v1/jobs/SC-1/diff").body(Body::empty()).unwrap(),
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
        json!({"id": "x1", "jobId": a.id, "cmd": "ask", "args": {"text": "which db?"}, "at": 0});

    let (s, v) = call(app.clone(), post(&hooks_only, ask.to_string())).await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::FORBIDDEN, json!("scope"))
    );

    let mut other = ask.clone();
    other["jobId"] = json!("someone-else");
    let (s, v) = call(app.clone(), post(&token, other.to_string())).await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::FORBIDDEN, json!("wrong_job"))
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
    let stage = json!({"id": "x2", "jobId": a.id, "cmd": "stage.enter", "args": {"stage": "spec"}, "at": 0});
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
        Request::post("/api/v1/jobs")
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
        (&v["job"]["name"], &v["job"]["ticket"], &v["job"]["branch"]),
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
        (&v["job"]["name"], &v["job"]["branch"]),
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
        (&v["job"]["ticket"], &v["job"]["branch"]),
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
    let stages = feature["stages"].as_array().unwrap();
    assert!(
        stages.iter().all(|s| s.get("label").is_none()),
        "{stages:?}"
    );

    let bugfix = v["flows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["flow"] == "bugfix")
        .expect("the bugfix flow is listed");
    assert_eq!(bugfix["stages"][0]["id"], "analysis");
    let fix = bugfix["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "implement")
        .unwrap();
    assert_eq!(fix["label"], "Fix");
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
            if v["type"] == "job" && v["job"]["status"] == "paused" {
                return v;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(msg["job"]["name"], "SC-1");
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
    let mut b = Job::new("SC-2", 2);
    b.launch = Some(vec!["rt".into()]);
    b.workdir = Some(wt.to_string_lossy().into_owned());
    d.create_job(b).await.unwrap();
    let local =
        xherd_api::router(d).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));
    let req = || {
        get("/api/v1/jobs/SC-2/findings")
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
    for uri in ["/api/v1/state", "/api/v1/jobs"] {
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

#[tokio::test]
async fn companion_routes_are_gone() {
    let (_dir, d, _a) = setup().await;
    let local = xherd_api::router(d.clone())
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));
    // Now only the generic action route matches: no such action.
    let req = Request::post("/api/v1/jobs/SC-1/open")
        .header("host", "127.0.0.1:4740")
        .body(Body::empty())
        .unwrap();
    let (s, v) = call(local.clone(), req).await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::NOT_FOUND, json!("unknown_action"))
    );
    let req = get("/api/v1/companion/next?wait=0")
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(local.clone(), req).await.0, StatusCode::NOT_FOUND);

    // The test agent has no worktree.
    let req = get("/api/v1/jobs/SC-1/open-targets")
        .body(Body::empty())
        .unwrap();
    let (s, v) = call(local, req).await;
    assert!(s.is_client_error(), "{s}");
    assert_eq!(v["code"], "no_workspace");
}

fn post_json(uri: &str, body: Value) -> Request<Body> {
    Request::post(uri)
        .header("host", "127.0.0.1:4740")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn terminal_handler_script_is_served_for_the_host() {
    let (dir, d, _a) = setup().await;
    let local = xherd_api::router(d.clone())
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));
    let (s, h, body) = fetch(local.clone(), "/api/v1/open-in/terminal-handler.sh").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["content-type"], "text/plain; charset=utf-8");
    assert_eq!(h["cache-control"], "no-store");
    let script = String::from_utf8(body).unwrap();
    let root = dir.path().join("ws").join("worktrees");
    assert!(script.contains(&format!("'{}'", root.display())), "root");
    assert!(script.contains("'http://127.0.0.1:4740'"), "report url");
    assert!(script.contains(&format!("'{}'", open_in::HANDLER_VERSION)));
    assert!(script.contains(xherd_core::BUILD_VERSION));
    assert!(!script.contains("@ROOT@") && !script.contains("@HANDLER@"));

    let req = Request::get("/api/v1/open-in/terminal-handler.sh")
        .header("host", "127.0.0.1:4740'$(x)")
        .body(Body::empty())
        .unwrap();
    // Not a local Host either, so the user check answers first.
    assert!(call(local, req).await.0.is_client_error());

    // The Host check on its own, for a request with the user token.
    let token = xherd_core::config::Config::load(&d.paths.config())
        .unwrap()
        .0
        .user_token
        .unwrap();
    let req = Request::get("/api/v1/open-in/terminal-handler.sh")
        .header("host", "evil host")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let remote =
        xherd_api::router(d).layer(MockConnectInfo(SocketAddr::from(([10, 0, 0, 9], 5555))));
    let (s, v) = call(remote, req).await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::BAD_REQUEST, json!("bad_host"))
    );
}

#[tokio::test]
async fn terminal_handler_reports_are_kept() {
    let (_dir, d, _a) = setup().await;
    let local = xherd_api::router(d.clone())
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));
    let view = |app: axum::Router| async move {
        call(app, get("/api/v1/open-in").body(Body::empty()).unwrap())
            .await
            .1
    };
    let v = view(local.clone()).await;
    assert_eq!(v["handler"]["installed"], Value::Null);
    assert_eq!(v["handler"]["shipped"], json!(open_in::HANDLER_VERSION));
    assert_eq!(v["native"], json!(true));

    // An agent with a worktree: no Terminal entry before a report.
    let mut a = Job::new("SC-2", 2);
    a.workdir = Some(format!("{}/app/sc-2", d.worktrees_root()));
    d.create_job(a).await.unwrap();
    let targets = |app: axum::Router| async move {
        let (s, v) = call(
            app,
            get("/api/v1/jobs/SC-2/open-targets")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["targets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert!(
        !targets(local.clone())
            .await
            .contains(&"terminal".to_string())
    );

    let rec = json!({"version": open_in::HANDLER_VERSION, "xherd": "0.3.0",
        "os": "linux", "terminals": ["konsole", "xterm"], "root": d.worktrees_root()});
    let (s, v) = call(
        local.clone(),
        post_json("/api/v1/open-in/terminal-handler", rec),
    )
    .await;
    assert_eq!((s, v), (StatusCode::OK, json!({"ok": true})));
    let v = view(local.clone()).await;
    assert_eq!(
        v["handler"]["installed"]["terminals"],
        json!(["konsole", "xterm"])
    );
    assert_eq!(v["handler"]["updateAvailable"], json!(false));
    assert_eq!(v["handler"]["rootChanged"], json!(false));
    assert!(
        targets(local.clone())
            .await
            .contains(&"terminal".to_string())
    );
    // Kept on disk, so a restarted daemon still knows it.
    assert!(d.paths.terminal_handler().exists());
    assert_eq!(d.handler_record().unwrap().os, "linux");

    // An older handler for another root (written directly: version 1 is the
    // oldest a report may carry): update, root changed.
    std::fs::write(
        d.paths.terminal_handler(),
        json!({"version": open_in::HANDLER_VERSION - 1, "xherd": "x", "os": "mac",
            "terminals": [], "root": "/a", "at": 1})
        .to_string(),
    )
    .unwrap();
    let v = view(local.clone()).await;
    assert_eq!(v["handler"]["updateAvailable"], json!(true));
    assert_eq!(v["handler"]["rootChanged"], json!(true));

    // Bad reports are refused.
    for bad in [
        json!({"version": 0, "xherd": "x", "os": "linux", "terminals": [], "root": "/a"}),
        json!({"version": 1, "xherd": "x", "os": "windows", "terminals": [], "root": "/a"}),
        json!({"version": 1, "xherd": "x", "os": "mac", "terminals": ["rm"], "root": "/a"}),
        json!({"version": 1, "xherd": "", "os": "mac", "terminals": [], "root": "/a"}),
        json!({"version": 1, "xherd": "x", "os": "mac", "terminals": [], "root": "/a\nb"}),
        json!({"installed": true}),
        json!({"hello": 1}),
    ] {
        let (s, _) = call(
            local.clone(),
            post_json("/api/v1/open-in/terminal-handler", bad.clone()),
        )
        .await;
        assert!(s.is_client_error(), "{bad}: {s}");
    }

    // Uninstalled: the record and the Terminal entry go.
    let (s, _) = call(
        local.clone(),
        post_json(
            "/api/v1/open-in/terminal-handler",
            json!({"installed": false}),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(!d.paths.terminal_handler().exists());
    assert_eq!(
        view(local.clone()).await["handler"]["installed"],
        Value::Null
    );
    assert!(!targets(local).await.contains(&"terminal".to_string()));
}

#[tokio::test]
async fn terminal_handler_install_is_native_only() {
    let (_dir, d, _a) = setup().await;
    // The container's daemon, without starting one (that would install the
    // pack into the real HOME).
    let mut c = d.clone();
    c.sandbox = true;
    let local =
        xherd_api::router(c).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));
    let token = xherd_core::config::Config::load(&d.paths.config())
        .unwrap()
        .0
        .user_token
        .unwrap();
    let req = Request::post("/api/v1/open-in/terminal-handler/install")
        .header("host", "127.0.0.1:4740")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let (s, v) = call(local.clone(), req).await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("native_only"))
    );
    let (s, _) = call(
        local,
        post_json(
            "/api/v1/open-in/terminal-handler/install",
            json!({"bogus": 1}),
        ),
    )
    .await;
    assert!(s.is_client_error(), "{s}");
}

/// R3: the old `/agents/…` routes answer `410 moved` for a job and `404`
/// for anything else.
#[tokio::test]
async fn old_agent_routes_are_gone() {
    let (_dir, d, _a) = setup().await;
    let app =
        xherd_api::router(d.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
    for (method, uri, location) in [
        ("POST", "/api/v1/agents/pause-all", "/api/v1/jobs/pause-all"),
        ("GET", "/api/v1/agents/SC-1", "/api/v1/jobs/SC-1"),
        (
            "POST",
            "/api/v1/agents/SC-1/pause",
            "/api/v1/jobs/SC-1/pause",
        ),
        (
            "GET",
            "/api/v1/agents/SC-1/ticket-status",
            "/api/v1/jobs/SC-1/ticket-status",
        ),
    ] {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:4740")
            .body(Body::empty())
            .unwrap();
        let (s, v) = call(app.clone(), req).await;
        assert_eq!(
            (s, v["code"].clone(), v["location"].clone()),
            (StatusCode::GONE, json!("moved"), json!(location)),
            "{method} {uri}"
        );
    }
    let (s, v) = call(
        app,
        get("/api/v1/agents/NOPE/diff").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::NOT_FOUND, json!("unknown_job"))
    );
}

fn send(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let b = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1:4740");
    match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    }
}

#[tokio::test]
async fn saved_agent_routes() {
    let (_dir, d, _a) = setup().await;
    let app =
        xherd_api::router(d.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
    let new = json!({"name": "reviewer", "runtime": "claude-code"});
    let (s, v) = call(
        app.clone(),
        send("POST", "/api/v1/agents", Some(new.clone())),
    )
    .await;
    assert_eq!(
        (s, v["version"].clone(), v["status"].clone()),
        (StatusCode::OK, json!(1), json!("idle"))
    );
    let id = v["id"].as_str().unwrap().to_string();
    assert_eq!(v["liveJobs"], 0);
    // refusals have their codes and statuses
    for (body, status, code) in [
        (new.clone(), StatusCode::CONFLICT, "name_taken"),
        (
            json!({"name": "x", "runtime": "claude-code", "fallbackRuntime": "claude-code"}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "bad_fallback",
        ),
        (
            json!({"name": "Bad Name", "runtime": "claude-code"}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "bad_name",
        ),
    ] {
        let (s, v) = call(app.clone(), send("POST", "/api/v1/agents", Some(body))).await;
        assert_eq!((s, v["code"].clone()), (status, json!(code)));
    }
    let (s, v) = call(
        app.clone(),
        get("/api/v1/agents").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(
        (s, v["agents"].as_array().unwrap().len()),
        (StatusCode::OK, 1)
    );
    let (_, st) = call(
        app.clone(),
        get("/api/v1/state").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(st["agents"][0]["name"], "reviewer");
    assert_eq!(st["jobs"].as_array().unwrap().len(), 1, "jobs stay jobs");

    // by name and by id; edit; versions; archive
    let (s, v) = call(
        app.clone(),
        get("/api/v1/agents/reviewer").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!((s, v["id"].clone()), (StatusCode::OK, json!(id)));
    let mut edit = new.clone();
    edit["description"] = json!("more");
    let (s, v) = call(
        app.clone(),
        send("PUT", &format!("/api/v1/agents/{id}"), Some(edit.clone())),
    )
    .await;
    assert_eq!((s, v["version"].clone()), (StatusCode::OK, json!(2)));
    edit["name"] = json!("renamed");
    let (s, v) = call(
        app.clone(),
        send("PUT", "/api/v1/agents/reviewer", Some(edit)),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("name_fixed"))
    );
    let (s, v) = call(
        app.clone(),
        send("PUT", "/api/v1/agents/reviewer", Some(json!({"nope": 1}))),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::BAD_REQUEST, json!("bad_request"))
    );
    let (_, v) = call(
        app.clone(),
        get("/api/v1/agents/reviewer/versions")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(v["versions"].as_array().unwrap().len(), 2);
    let (_, v) = call(
        app.clone(),
        get("/api/v1/agents/reviewer/jobs")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(v["jobs"], json!([]));
    // memory (R28)
    let (_, v) = call(
        app.clone(),
        get("/api/v1/agents/reviewer/memory")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(v, json!({"text": "", "version": 0}));
    let (s, v) = call(
        app.clone(),
        send(
            "PUT",
            "/api/v1/agents/reviewer/memory",
            Some(json!({"text": "Use qa/."})),
        ),
    )
    .await;
    assert_eq!(
        (s, v),
        (StatusCode::OK, json!({"text": "Use qa/.", "version": 1}))
    );
    let (s, v) = call(
        app.clone(),
        send(
            "PUT",
            "/api/v1/agents/reviewer/memory",
            Some(json!({"text": "x".repeat(4001)})),
        ),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::UNPROCESSABLE_ENTITY, json!("bad_memory"))
    );
    let (_, v) = call(
        app.clone(),
        send("POST", "/api/v1/agents/reviewer/archive", None),
    )
    .await;
    assert_eq!(v["status"], "archived");
    let (_, v) = call(
        app.clone(),
        send("POST", "/api/v1/agents/reviewer/unarchive", None),
    )
    .await;
    assert_eq!(v["status"], "idle");

    // export and import
    let (s, v) = call(
        app.clone(),
        get("/api/v1/agents/export?name=reviewer")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        (s, v["agents"].as_array().unwrap().len()),
        (StatusCode::OK, 1)
    );
    let mut copy = v.clone();
    copy["agents"][0]["name"] = json!("copy");
    let (_, dry) = call(
        app.clone(),
        send("POST", "/api/v1/agents/import?dryRun=1", Some(copy.clone())),
    )
    .await;
    assert_eq!(dry["agents"][0]["agent"]["name"], "copy");
    assert_eq!(
        d.agents().await.unwrap().len(),
        1,
        "a dry run writes nothing"
    );
    let (s, v) = call(
        app.clone(),
        send("POST", "/api/v1/agents/import", Some(copy)),
    )
    .await;
    assert_eq!((s, v["added"].clone()), (StatusCode::OK, json!(1)));

    // a job with the same name as no agent still gets the 410, an agent route
    // that does not exist the 404, and a job name on an agent path the 410
    let (s, v) = call(
        app.clone(),
        get("/api/v1/agents/reviewer/ticket")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::NOT_FOUND, json!("no_such_route"))
    );
    for (method, uri, location) in [
        ("GET", "/api/v1/agents/SC-1", "/api/v1/jobs/SC-1"),
        (
            "POST",
            "/api/v1/agents/SC-1/archive",
            "/api/v1/jobs/SC-1/archive",
        ),
        (
            "GET",
            "/api/v1/agents/SC-1/versions",
            "/api/v1/jobs/SC-1/versions",
        ),
    ] {
        let (s, v) = call(app.clone(), send(method, uri, None)).await;
        assert_eq!(
            (s, v["code"].clone(), v["location"].clone()),
            (StatusCode::GONE, json!("moved"), json!(location)),
            "{method} {uri}"
        );
    }
    let (s, v) = call(
        app.clone(),
        send("POST", "/api/v1/agents/NOPE/archive", None),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::NOT_FOUND, json!("unknown_job"))
    );

    // delete
    let (s, _) = call(app.clone(), send("DELETE", "/api/v1/agents/copy", None)).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = call(app, send("DELETE", "/api/v1/agents/copy", None)).await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::NOT_FOUND, json!("unknown_job"))
    );
}

/// R25: the skills library routes and their refusals.
#[tokio::test]
async fn skill_routes() {
    let (_dir, d, _a) = setup().await;
    let app =
        xherd_api::router(d.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
    let md = "---\nname: lint-fix\ndescription: Fixes lint\n---\nSteps\n";
    let (s, v) = call(
        app.clone(),
        send(
            "POST",
            "/api/v1/skills",
            Some(json!({"name": "lint-fix", "text": md})),
        ),
    )
    .await;
    assert_eq!(
        (s, v["description"].clone()),
        (StatusCode::OK, json!("Fixes lint"))
    );
    let (s, v) = call(
        app.clone(),
        send(
            "POST",
            "/api/v1/skills",
            Some(json!({"name": "lint-fix", "text": md})),
        ),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("name_taken"))
    );
    let (s, v) = call(
        app.clone(),
        send(
            "POST",
            "/api/v1/skills",
            Some(json!({"name": "x", "text": "no header"})),
        ),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::UNPROCESSABLE_ENTITY, json!("bad_skill"))
    );
    let (s, v) = call(
        app.clone(),
        get("/api/v1/skills").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(
        (s, v["skills"][0]["name"].clone()),
        (StatusCode::OK, json!("lint-fix"))
    );
    let (s, v) = call(
        app.clone(),
        get("/api/v1/skills/lint-fix").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!((s, v["text"].clone()), (StatusCode::OK, json!(md)));
    // an agent needs the skill to exist; the skill cannot go while it is used
    let agent = |skills: Value| {
        json!({"name": "rev", "runtime": "claude-code",
               "skills": skills})
    };
    let (s, v) = call(
        app.clone(),
        send("POST", "/api/v1/agents", Some(agent(json!(["nope"])))),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::UNPROCESSABLE_ENTITY, json!("unknown_skill"))
    );
    let (s, _) = call(
        app.clone(),
        send("POST", "/api/v1/agents", Some(agent(json!(["lint-fix"])))),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = call(app.clone(), send("DELETE", "/api/v1/skills/lint-fix", None)).await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("skill_in_use"))
    );
    let (s, v) = call(app, send("DELETE", "/api/v1/skills/nope", None)).await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::NOT_FOUND, json!("no_such_skill"))
    );
}

/// Mark done, suggestion and save-as-agent (R30…R32).
#[tokio::test]
async fn mark_done_and_save_as_agent_routes() {
    let (_dir, d, _a) = setup().await;
    let app =
        xherd_api::router(d.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
    let form = json!({"agent": {"name": "shop", "description": "Shop",
        "runtime": "claude-code", "instructions": "Run the linter."}});
    // SC-1 is running: it can be neither marked done nor accepted.
    let (s, v) = call(
        app.clone(),
        send("POST", "/api/v1/jobs/SC-1/mark-done", None),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("not_allowed"))
    );
    let (s, v) = call(
        app.clone(),
        send(
            "POST",
            "/api/v1/jobs/SC-1/suggestion/accept",
            Some(form.clone()),
        ),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("no_suggestion"))
    );
    let (s, v) = call(
        app.clone(),
        send(
            "POST",
            "/api/v1/jobs/SC-1/save-as-agent",
            Some(json!({"nope": 1})),
        ),
    )
    .await;
    // Like the agent routes: a body of the wrong shape is axum's 422.
    assert_eq!((s, v), (StatusCode::UNPROCESSABLE_ENTITY, Value::Null));
    let (s, v) = call(
        app.clone(),
        send(
            "POST",
            "/api/v1/jobs/SC-1/save-as-agent",
            Some(form.clone()),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["agent"]["name"], "shop");
    let id = v["job"]["currentAgent"].as_str().unwrap().to_string();
    assert_eq!(v["job"]["agents"][id.as_str()]["name"], "shop");
    assert_eq!(v["job"]["defaultAgent"], id.as_str());
    assert_eq!(v["job"]["pinned"], true);
    assert!(v["job"].get("agent").is_none());
    let (s, v) = call(
        app.clone(),
        send("POST", "/api/v1/jobs/SC-1/save-as-agent", Some(form)),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("has_agent"))
    );
    // Dismissing with nothing to dismiss is a no-op.
    let (s, _) = call(
        app,
        send("POST", "/api/v1/jobs/SC-1/suggestion/dismiss", None),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn flow_assignment_routes() {
    let (_dir, d, _a) = setup().await;
    let app =
        xherd_api::router(d.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
    let (s, _) = call(
        app.clone(),
        send(
            "POST",
            "/api/v1/agents",
            Some(json!({"name": "architect", "runtime": "claude-code"})),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = call(
        app.clone(),
        get("/api/v1/flows").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["flows"].as_array().unwrap().len(), 2);
    let url = "/api/v1/flows/xherd/feature/assignments";
    let put = |body| send("PUT", url, Some(body));
    let (s, v) = call(
        app.clone(),
        put(json!({"assignments": [{"stage": "design", "agentId": "architect", "human": true}]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let design = v["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "design")
        .unwrap();
    assert_eq!(
        (
            design["agent"].clone(),
            design["human"].clone(),
            design["assigned"].clone()
        ),
        (json!("architect"), json!(true), json!(true))
    );
    let (s, v) = call(app.clone(), get(url).body(Body::empty()).unwrap()).await;
    assert_eq!(
        (s, v["assignments"].as_array().unwrap().len()),
        (StatusCode::OK, 1)
    );
    let (s, v) = call(
        app.clone(),
        get("/api/v1/flows/xherd/feature")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!((s, v["flow"].clone()), (StatusCode::OK, json!("feature")));
    // refusals and their statuses
    for (body, status, code) in [
        (
            json!({"assignments": [{"stage": "nope", "agentId": "architect"}]}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "unknown_stage",
        ),
        (
            json!({"assignments": [{"stage": "design", "agentId": "ghost"}]}),
            StatusCode::NOT_FOUND,
            "unknown_agent",
        ),
        (
            json!({"assignments": [{"stage": "design", "agentId": "architect"}, {"stage": "design", "agentId": "architect"}]}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "bad_assignment",
        ),
    ] {
        let (s, v) = call(app.clone(), put(body)).await;
        assert_eq!((s, v["code"].clone()), (status, json!(code)));
    }
    let (s, v) = call(
        app.clone(),
        send(
            "PUT",
            "/api/v1/flows/xherd/nope/assignments",
            Some(json!({"assignments": []})),
        ),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::UNPROCESSABLE_ENTITY, json!("unknown_flow"))
    );
    // an assigned agent cannot be deleted (409)
    let (s, v) = call(
        app.clone(),
        send("DELETE", "/api/v1/agents/architect", None),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("agent_assigned"))
    );
}

/// A local git repo holding one host plugin, as a `file://` source.
fn plugin_repo() -> (tempfile::TempDir, String) {
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(
        repo.path().join("xherd-plugin.json"),
        json!({"name": "gitea", "kind": "host", "version": "1.0.0"}).to_string(),
    )
    .unwrap();
    for args in [
        vec!["init", "--quiet", "-b", "main"],
        vec!["add", "--all"],
        vec!["commit", "--quiet", "-m", "x"],
    ] {
        let out = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(&args)
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }
    let url = format!("file://{}", repo.path().display());
    (repo, url)
}

#[tokio::test]
async fn plugin_routes() {
    let (_dir, d, _a) = setup_opts(|o| o.allow_local_plugin_sources = true, |_| {}).await;
    let (_repo, url) = plugin_repo();
    let local =
        xherd_api::router(d.clone()).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
    let remote = xherd_api::router(d.clone())
        .layer(MockConnectInfo(SocketAddr::from(([192, 168, 1, 5], 5555))));

    // The user only: no rights from another host without a token, nor with a
    // job's token (R10).
    let job_t = mint_token(&d.store, TOKEN_AGENT, Some("x"), &[SCOPE_XAP], 1)
        .await
        .unwrap();
    for (method, uri, body) in [
        ("GET", "/api/v1/plugins", None),
        ("POST", "/api/v1/plugins/preview", Some(json!({"url": url}))),
        (
            "POST",
            "/api/v1/plugins/install",
            Some(json!({"url": url, "digest": "x"})),
        ),
        ("POST", "/api/v1/plugins/gitea/update", Some(json!({}))),
        ("DELETE", "/api/v1/plugins/gitea", None),
        ("POST", "/api/v1/admin/restart", None),
    ] {
        let (s, _) = call(remote.clone(), send(method, uri, body.clone())).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{method} {uri}");
        let mut req = send(method, uri, body);
        req.headers_mut()
            .insert("authorization", format!("Bearer {job_t}").parse().unwrap());
        let (s, _) = call(local.clone(), req).await;
        assert_eq!(
            s,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} with a job token"
        );
    }

    // Preview, then install what was previewed.
    let (s, v) = call(
        local.clone(),
        send("POST", "/api/v1/plugins/preview", Some(json!({"url": url}))),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["name"], "gitea");
    let digest = v["digest"].as_str().unwrap().to_string();
    let (s, v) = call(
        local.clone(),
        send("POST", "/api/v1/plugins/install", Some(json!({"url": url}))),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::BAD_REQUEST, json!("usage"))
    );
    let (s, v) = call(
        local.clone(),
        send(
            "POST",
            "/api/v1/plugins/install",
            Some(json!({"url": url, "digest": "stale"})),
        ),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("changed_since_preview"))
    );
    let (s, v) = call(
        local.clone(),
        send(
            "POST",
            "/api/v1/plugins/install",
            Some(json!({"url": url, "digest": digest})),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["name"], "gitea");

    // Listed with its source; a restart is needed to load it.
    let (s, v) = call(
        local.clone(),
        get("/api/v1/plugins").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["restartNeeded"], true);
    let recorded = v["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "gitea");
    // Not loaded until the restart, so only its record exists.
    assert!(recorded.is_none());

    // Update with no change shows an empty diff; built-ins cannot be removed.
    let (s, v) = call(
        local.clone(),
        send("POST", "/api/v1/plugins/gitea/update", Some(json!({}))),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["manifestDiff"], "");
    let (s, v) = call(
        local.clone(),
        send("DELETE", "/api/v1/plugins/claude-code", None),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::CONFLICT, json!("builtin_plugin"))
    );
    let (s, v) = call(local.clone(), send("DELETE", "/api/v1/plugins/nope", None)).await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::NOT_FOUND, json!("unknown_plugin"))
    );
    let (s, v) = call(
        local.clone(),
        send("DELETE", "/api/v1/plugins/gitea?forget=true", None),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["forgot"], true);

    // Restart: 202, and the daemon's waiter wakes.
    let (s, _) = call(local.clone(), send("POST", "/api/v1/admin/restart", None)).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    tokio::time::timeout(Duration::from_secs(5), d.restart_requested())
        .await
        .expect("restart requested");
}

#[tokio::test]
async fn plugin_install_off_is_forbidden() {
    let (_dir, d, _a) = setup_opts(
        |o| {
            o.allow_local_plugin_sources = true;
            o.plugin_install = false;
        },
        |_| {},
    )
    .await;
    let (_repo, url) = plugin_repo();
    let local = xherd_api::router(d).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1))));
    let (s, v) = call(
        local,
        send("POST", "/api/v1/plugins/preview", Some(json!({"url": url}))),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (StatusCode::FORBIDDEN, json!("plugin_install_off"))
    );
}
