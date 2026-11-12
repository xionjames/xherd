use std::path::Path;

use serde_json::{Map, Value, json};
use xherd_plugin::*;

fn write_plugin(dir: &Path, name: &str, manifest: Value, script: &str) {
    let d = dir.join(name);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join(MANIFEST_FILE), manifest.to_string()).unwrap();
    std::fs::write(d.join("run"), script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(d.join("run"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn manifest(name: &str) -> Value {
    json!({
        "name": name, "kind": "host", "version": "1.0.0", "timeoutSeconds": 2,
        "settings": [
            {"key": "base", "default": "main"},
            {"key": "draft", "type": "boolean", "default": false},
            {"key": "token", "secret": true, "required": true}
        ],
        "commands": {
            "echo": ["./run", "echo"], "fail": ["./run", "fail"],
            "slow": ["./run", "slow"], "junk": ["./run", "junk"]
        },
        "checks": [
            {"name": "ok", "argv": ["true"]},
            {"name": "bad", "argv": ["./run", "fail"], "optional": true, "fix": ["true"]}
        ]
    })
}

const SCRIPT: &str = r#"#!/bin/sh
case "$1" in
  echo) input=$(cat); printf '{"stdin": %s, "id": "%s", "ctx": %s}' "$input" "$XHERD_PLUGIN_ID" "$XHERD_CONTEXT_JSON" ;;
  fail) echo "bad token s3cr3t-value" >&2; exit 3 ;;
  slow) sleep 30 ;;
  junk) echo "not json" ;;
esac
"#;

fn ctx() -> Context {
    Context {
        settings: Map::from_iter([("base".into(), json!("develop"))]),
        secrets: Map::from_iter([("token".into(), json!("s3cr3t-value"))]),
        context: json!({"repo": "r"}),
        ..Context::default()
    }
}

#[test]
fn registry_overrides_builtins_and_lists_broken() {
    let d = tempfile::tempdir().unwrap();
    write_plugin(d.path(), "github", manifest("github"), SCRIPT);
    write_plugin(d.path(), "wrong-folder", manifest("other"), SCRIPT);
    let mut bad = manifest("badcmd");
    bad["commands"]["x"] = json!(["./missing"]);
    write_plugin(d.path(), "badcmd", bad, SCRIPT);
    std::fs::create_dir(d.path().join("not-a-plugin")).unwrap();

    let builtin =
        Manifest::parse(&json!({"name": "github", "kind": "host", "version": "0.1.0"}).to_string())
            .unwrap();
    let r = Registry::load(vec![builtin], d.path());
    let gh = r.get("github").unwrap();
    assert!(!gh.builtin(), "external overrides built-in");
    assert_eq!(gh.manifest.version, "1.0.0");
    let names: Vec<&str> = r.broken().iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, vec!["badcmd", "wrong-folder"]);
    assert!(r.broken()[0].error.contains("not found"));
    assert_eq!(r.of_kind(Kind::Host).count(), 1);
}

#[test]
fn settings_resolution_order_and_secrets() {
    let m: Manifest = serde_json::from_value(manifest("p")).unwrap();
    let global = Map::from_iter([
        ("base".into(), json!("develop")),
        ("unknown".into(), json!(1)),
    ]);
    let profile = Map::from_iter([
        ("draft".into(), json!(true)),
        ("token".into(), json!("leak")),
    ]);
    let s = resolve_settings(&m, Some(&global), Some(&profile));
    assert_eq!(
        Value::Object(s.clone()),
        json!({"base": "develop", "draft": true})
    );
    let secrets = resolve_secrets(
        &m,
        &Map::from_iter([("token".into(), json!("t")), ("other".into(), json!("x"))]),
    );
    assert_eq!(Value::Object(secrets.clone()), json!({"token": "t"}));
    assert_eq!(missing_required(&m, &s, &Map::new()), vec!["token"]);
    assert!(missing_required(&m, &s, &secrets).is_empty());
}

#[tokio::test]
async fn command_protocol() {
    let d = tempfile::tempdir().unwrap();
    write_plugin(d.path(), "p", manifest("p"), SCRIPT);
    let r = Registry::load(vec![], d.path());
    let p = r.get("p").unwrap();
    let c = ctx();

    let v = run_command(p, "echo", json!({"branch": "SC-1"}), &c)
        .await
        .unwrap();
    assert_eq!(v["id"], "p");
    assert_eq!(v["ctx"]["repo"], "r");
    assert_eq!(v["stdin"]["command"], "echo");
    assert_eq!(v["stdin"]["input"]["branch"], "SC-1");
    assert_eq!(v["stdin"]["settings"]["base"], "develop");
    assert_eq!(v["stdin"]["secrets"]["token"], "s3cr3t-value");

    match run_command(p, "fail", Value::Null, &c).await.unwrap_err() {
        PluginError::Failed { code, stderr, .. } => {
            assert_eq!(code, Some(3));
            assert!(
                stderr.contains("***") && !stderr.contains("s3cr3t"),
                "{stderr}"
            );
        }
        e => panic!("{e}"),
    }
    assert!(matches!(
        run_command(p, "slow", Value::Null, &c).await,
        Err(PluginError::Timeout { .. })
    ));
    assert!(matches!(
        run_command(p, "junk", Value::Null, &c).await,
        Err(PluginError::BadJson { .. })
    ));
    assert!(matches!(
        run_command(p, "nope", Value::Null, &c).await,
        Err(PluginError::Unknown { .. })
    ));

    #[derive(serde::Deserialize, Debug)]
    #[allow(dead_code)]
    struct Pr {
        url: String,
    }
    assert!(matches!(
        run_typed::<Pr>(p, "echo", Value::Null, &c).await,
        Err(PluginError::Shape { .. })
    ));

    let checks = run_checks(p, &c).await;
    assert_eq!(checks.len(), 2);
    assert!(checks[0].ok);
    assert!(!checks[1].ok && checks[1].optional && checks[1].fixable);
}

#[tokio::test]
async fn host_pr_for_branch_builtin_and_external() {
    let gh = builtins().into_iter().find(|m| m.name == "github").unwrap();
    assert_eq!(gh.kind, Kind::Host);
    assert_eq!(
        Value::Object(resolve_settings(&gh, None, None)),
        json!({"gh": "gh"})
    );

    // An external `github` replaces the built-in one through the command protocol.
    let d = tempfile::tempdir().unwrap();
    let mut m = manifest("github");
    m["settings"] = json!([]);
    m["commands"] = json!({"pr-for-branch": ["./run"]});
    let script = r#"#!/bin/sh
input=$(cat)
case "$input" in
  *'"branch":"SC-1"'*) echo '{"url": "https://example.test/pr/1"}' ;;
  *'"branch":"SC-2"'*) echo 'null' ;;
  *) echo '{"url": 3}' ;;
esac
"#;
    write_plugin(d.path(), "github", m, script);
    let r = Registry::load(builtins(), d.path());
    let host = host::Host {
        plugin: r.get("github").unwrap().clone(),
        ctx: Context::default(),
    };
    let wt = d.path();
    assert_eq!(
        host.pr_for_branch(wt, "SC-1").await.unwrap().as_deref(),
        Some("https://example.test/pr/1")
    );
    assert_eq!(host.pr_for_branch(wt, "SC-2").await.unwrap(), None);
    assert!(matches!(
        host.pr_for_branch(wt, "SC-3").await,
        Err(PluginError::Shape { .. })
    ));
}

#[test]
fn builtin_toolchains_parse_and_match_the_plugins_folder() {
    let tcs: Vec<_> = builtins()
        .into_iter()
        .filter(|m| m.kind == Kind::Toolchain)
        .map(|m| {
            (
                m.name.clone(),
                xherd_plugin::toolchain::Toolchain::of(&m).unwrap(),
            )
        })
        .collect();
    let names: Vec<&str> = tcs.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["sdkman", "nvm", "uv", "go", "rustup"]);
    // Every folder under plugins/ is a built-in.
    let mut dirs: Vec<String> =
        std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../plugins"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
    dirs.sort();
    let mut all: Vec<String> = builtins().into_iter().map(|m| m.name).collect();
    all.sort();
    assert_eq!(dirs, all);

    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join(".sdkmanrc"), "java=21.0.2-tem\n").unwrap();
    std::fs::write(d.path().join(".nvmrc"), "20\n").unwrap();
    std::fs::write(
        d.path().join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.80.0\"\n",
    )
    .unwrap();
    let plan = xherd_plugin::toolchain::plan(d.path(), &tcs, true, &Default::default());
    let got: Vec<(&str, &str)> = plan
        .steps
        .iter()
        .map(|s| (s.tool.as_str(), s.version.as_str()))
        .collect();
    assert_eq!(
        got,
        [("java", "21.0.2-tem"), ("node", "20"), ("rust", "1.80.0")]
    );
}

/// The runtime plugins' container installs name the versions the image is
/// built with (and xherd was checked against): the Dockerfile's `ARG`s.
#[test]
fn runtime_installs_use_the_image_versions() {
    let dockerfile = include_str!("../../../deploy/docker/Dockerfile");
    let arg = |name: &str| {
        dockerfile
            .lines()
            .find_map(|l| l.strip_prefix(&format!("ARG {name}=")))
            .unwrap_or_else(|| panic!("no ARG {name}"))
            .trim()
            .to_string()
    };
    for (plugin, tool, package, arg_name) in [
        (
            "claude-code",
            "claude",
            "@anthropic-ai/claude-code",
            "CLAUDE_CODE_VERSION",
        ),
        ("opencode", "opencode", "opencode-ai", "OPENCODE_VERSION"),
        ("codex", "codex", "@openai/codex", "CODEX_VERSION"),
    ] {
        let m = xherd_plugin::builtins()
            .into_iter()
            .find(|m| m.name == plugin)
            .unwrap();
        let r = m.requires.iter().find(|r| r.name == tool).unwrap();
        let install = r.install.as_ref().unwrap().join(" ");
        assert!(
            install.ends_with(&format!("{package}@{}", arg(arg_name))),
            "{plugin}: {install}"
        );
    }
}
