//! `POST` and `GET /api/v1/jobs/:id/files` (#192, R1, R3, R5, R6).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use xherd_core::new::NewJob;
use xherd_core::paths::Paths;
use xherd_daemon::settings::ConfigPatch;
use xherd_daemon::*;

const BOUNDARY: &str = "xherd-test-boundary";

fn git(dir: &Path, args: &[&str]) {
    let o = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}");
}

struct T {
    d: Daemon,
    wd: PathBuf,
    _dirs: Vec<tempfile::TempDir>,
}

impl T {
    fn app(&self) -> axum::Router {
        xherd_api::router(self.d.clone())
            .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))))
    }

    fn uploaded(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.wd.join(".xherd/uploads"))
            .map(|r| {
                r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

async fn setup() -> T {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    git(
        repo.path(),
        &["commit", "-q", "--allow-empty", "-m", "base"],
    );
    let paths = Paths::at(home.path());
    paths.ensure().unwrap();
    xherd_core::config::Config {
        workspace: Some(ws.path().to_string_lossy().into_owned()),
        ..Default::default()
    }
    .save(&paths.config())
    .unwrap();
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    o.scheduler = None;
    // Never the real Claude Code config: a fixture that trusts the workspace.
    let trust = home.path().join("claude.json");
    std::fs::write(
        &trust,
        json!({"projects": {ws.path().to_string_lossy(): {"hasTrustDialogAccepted": true}}})
            .to_string(),
    )
    .unwrap();
    let rt = o
        .builtin_plugins
        .iter_mut()
        .find(|m| m.name == "claude-code")
        .unwrap();
    rt.data.get_mut("terminal").unwrap()["trust"]["file"] = trust.to_string_lossy().into();
    let d = Daemon::start(paths, o).await.unwrap();
    let job = d
        .new_job(NewJob {
            name: "up".into(),
            ticket: None,
            ticket_key: None,
            slug: None,
            repo: repo.path().to_string_lossy().into_owned(),
            agent: None,
            agents: Default::default(),
            base: None,
            flow: None,
            prompt: Some("add login".into()),
            autonomy: None,
            permission_mode: None,
            runtime: None,
            model: None,
            new_project: None,
        })
        .await
        .unwrap();
    let wd = PathBuf::from(job.workdir.unwrap());
    T {
        d,
        wd,
        _dirs: vec![home, ws, repo],
    }
}

fn multipart(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut b = Vec::new();
    for (name, bytes) in files {
        b.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        b.extend_from_slice(bytes);
        b.extend_from_slice(b"\r\n");
    }
    b.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    b
}

fn post_files(job: &str, body: Vec<u8>) -> Request<Body> {
    Request::post(format!("/api/v1/jobs/{job}/files"))
        .header("host", "127.0.0.1:4740")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(body))
        .unwrap()
}

fn get_file(job: &str, path: &str) -> Request<Body> {
    Request::get(format!("/api/v1/jobs/{job}/files"))
        .header("host", "127.0.0.1:4740")
        .body(Body::empty())
        .map(|mut r| {
            *r.uri_mut() = axum::http::Uri::builder()
                .path_and_query(format!(
                    "/api/v1/jobs/{job}/files?path={}",
                    path.replace('/', "%2F").replace(' ', "%20")
                ))
                .build()
                .unwrap();
            r
        })
        .unwrap()
}

async fn call(
    app: axum::Router,
    req: Request<Body>,
) -> (StatusCode, Vec<(String, String)>, Vec<u8>) {
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string()))
        .collect();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes.to_vec())
}

fn json_of(b: &[u8]) -> Value {
    serde_json::from_slice(b).unwrap_or(Value::Null)
}

#[tokio::test]
async fn a_6_mib_upload_passes_the_global_body_limit_and_is_stored() {
    let t = setup().await;
    let big = vec![7u8; 6 * 1024 * 1024];
    let body = multipart(&[("big.bin", &big), ("../note.txt", b"hello")]);
    let (s, _, b) = call(t.app(), post_files("up", body)).await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
    let v = json_of(&b);
    let up = t.wd.join(".xherd/uploads");
    assert_eq!(
        v["paths"],
        json!([
            up.join("big.bin").to_string_lossy(),
            up.join("note.txt").to_string_lossy()
        ])
    );
    assert_eq!(std::fs::read(up.join("big.bin")).unwrap().len(), big.len());
    assert_eq!(t.uploaded(), ["big.bin", "note.txt"]);
}

#[tokio::test]
async fn too_large_is_413_and_stores_nothing() {
    let t = setup().await;
    t.d.save_config(ConfigPatch {
        upload_max_bytes: Some(1024),
        ..Default::default()
    })
    .unwrap();
    // The first file fits, the second does not: the first is rolled back.
    let body = multipart(&[("small.txt", b"ok"), ("big.bin", &[1u8; 2048])]);
    let (s, _, b) = call(t.app(), post_files("up", body)).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(json_of(&b)["code"], "too_large");
    assert!(t.uploaded().is_empty(), "{:?}", t.uploaded());
}

#[tokio::test]
async fn a_request_without_files_is_refused() {
    let t = setup().await;
    let (s, _, _) = call(t.app(), post_files("up", multipart(&[]))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = call(t.app(), post_files("nope", multipart(&[("a", b"x")]))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn download_streams_a_file_with_safe_headers() {
    let t = setup().await;
    std::fs::write(t.wd.join("out.html"), b"<b>hi</b>").unwrap();
    let (s, h, b) = call(t.app(), get_file("up", "out.html")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b, b"<b>hi</b>");
    let header = |n: &str| h.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    assert_eq!(header("content-type"), Some("application/octet-stream"));
    assert_eq!(header("content-length"), Some("9"));
    assert_eq!(
        header("content-disposition"),
        Some("attachment; filename=\"out.html\"")
    );
    assert_eq!(header("x-content-type-options"), Some("nosniff"));

    // An absolute path inside the worktree works too (what the terminal prints).
    let abs = t.wd.canonicalize().unwrap().join("out.html");
    let (s, _, b) = call(t.app(), get_file("up", &abs.to_string_lossy())).await;
    assert_eq!((s, b.as_slice()), (StatusCode::OK, &b"<b>hi</b>"[..]));
}

#[tokio::test]
async fn download_refuses_missing_and_outside_paths() {
    let t = setup().await;
    let (s, _, b) = call(t.app(), get_file("up", "missing.txt")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(json_of(&b)["code"], "no_such_file");

    let (s, _, b) = call(t.app(), get_file("up", "../../../../../../etc/hosts")).await;
    assert!(
        matches!(s, StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND),
        "{s}"
    );
    assert!(!String::from_utf8_lossy(&b).contains("localhost"));

    let (s, _, _) = call(t.app(), get_file("up", "/etc/hosts")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
