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

/// The CLI each built-in runtime installs: (plugin, tool).
const RUNTIMES: [(&str, &str); 3] = [
    ("claude-code", "claude"),
    ("opencode", "opencode"),
    ("codex", "codex"),
];

fn runtime_requirement(plugin: &str, tool: &str) -> xherd_plugin::manifest::Requirement {
    xherd_plugin::builtins()
        .into_iter()
        .find(|m| m.name == plugin)
        .unwrap()
        .requires
        .into_iter()
        .find(|r| r.name == tool)
        .unwrap()
}

/// The runtime plugins pin their CLI in the manifest: a version, a digest for
/// each CPU, and an install that reads them from the environment (R2, R10).
#[test]
fn runtime_installs_are_pinned_per_arch() {
    for (plugin, tool) in RUNTIMES {
        let r = runtime_requirement(plugin, tool);
        let version = r
            .version
            .clone()
            .unwrap_or_else(|| panic!("{plugin}: no version"));
        assert!(
            xherd_plugin::manifest::version_in(&version) == Some(version.as_str()),
            "{plugin}: {version}"
        );
        for arch in ["x86_64", "aarch64"] {
            let sum = r
                .sha256
                .get(arch)
                .unwrap_or_else(|| panic!("{plugin}: no {arch}"));
            assert!(sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        let argv = r
            .install
            .as_ref()
            .unwrap_or_else(|| panic!("{plugin}: no install"));
        assert_eq!(&argv[..2], ["sh", "-c"], "{plugin}");
        let script = &argv[2];
        assert_eq!(argv.len(), 3, "{plugin}");
        for var in [
            "XHERD_INSTALL_VERSION",
            "XHERD_INSTALL_ARCH",
            "XHERD_INSTALL_SHA256",
        ] {
            assert!(script.contains(var), "{plugin}: {var}");
        }
        assert!(!script.contains("npm"), "{plugin}: {script}");
        assert!(
            !script.contains(&version),
            "{plugin}: literal version in the script"
        );
        assert!(
            script.contains("sha256sum -c"),
            "{plugin}: digest not checked"
        );
    }
}

/// Runs a runtime's install script with a temporary `HOME` and `TMPDIR`, a
/// fake `curl` that serves `artifact`, and returns (exit ok, urls asked,
/// stray files left in the working directory or `TMPDIR`).
#[cfg(target_os = "linux")]
fn run_install(
    plugin: &str,
    tool: &str,
    home: &Path,
    artifact: &Path,
    sha256: &str,
) -> (bool, String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;
    let r = runtime_requirement(plugin, tool);
    let argv = r.install.clone().unwrap();
    let fake = tempfile::tempdir().unwrap();
    let urls = fake.path().join("urls");
    let curl = fake.path().join("curl");
    std::fs::write(
        &curl,
        format!(
            "#!/bin/sh\nout=\nfor a in \"$@\"; do [ \"$prev\" = -o ] && out=$a; prev=$a; done\n\
             echo \"$a\" >> '{}'\ncp '{}' \"$out\"\n",
            urls.display(),
            artifact.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let arch = std::env::consts::ARCH;
    let mut env = r.install_env(arch);
    env.retain(|(k, _)| k != "XHERD_INSTALL_SHA256");
    env.push(("XHERD_INSTALL_SHA256".into(), sha256.into()));
    let out = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", fake.path().display()))
        .env("HOME", home)
        .env("TMPDIR", tmp.path())
        .envs(env)
        .current_dir(cwd.path())
        .output()
        .unwrap();
    let stray = |d: &Path| -> Vec<String> {
        std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    };
    let mut stray_files = stray(cwd.path());
    stray_files.extend(stray(tmp.path()));
    (
        out.status.success(),
        std::fs::read_to_string(&urls).unwrap_or_default(),
        stray_files,
    )
}

/// The install writes only under `$HOME`, leaves no download behind, uses the
/// version and CPU it is given, and refuses an artifact with the wrong digest.
#[cfg(target_os = "linux")]
#[test]
fn runtime_installs_write_only_under_home() {
    let arch = std::env::consts::ARCH;
    let fixtures = tempfile::tempdir().unwrap();
    // Claude Code: the download is the binary; `claude install <version>` places it
    // (the fake checks it is asked for the pinned version).
    let claude = fixtures.path().join("claude");
    std::fs::write(
        &claude,
        "#!/bin/sh\n[ \"$1\" = install ] && [ \"$2\" = \"$XHERD_INSTALL_VERSION\" ] || exit 9\n\
         mkdir -p \"$HOME/.local/bin\" && echo placed > \"$HOME/.local/bin/claude\"\n",
    )
    .unwrap();
    // opencode and codex: a tarball with the binary inside.
    let tarball = |name: &str, inner: &str| {
        let dir = fixtures.path().join(format!("{name}.d"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(inner), "#!/bin/sh\necho fake\n").unwrap();
        let tgz = fixtures.path().join(format!("{name}.tar.gz"));
        let st = std::process::Command::new("tar")
            .arg("czf")
            .arg(&tgz)
            .arg("-C")
            .arg(&dir)
            .arg(inner)
            .status()
            .unwrap();
        assert!(st.success());
        tgz
    };
    let opencode = tarball("opencode", "opencode");
    let codex = tarball("codex", &format!("codex-{arch}-unknown-linux-musl"));

    for (plugin, tool, artifact, placed) in [
        ("claude-code", "claude", &claude, ".local/bin/claude"),
        ("opencode", "opencode", &opencode, ".opencode/bin/opencode"),
        ("codex", "codex", &codex, ".local/bin/codex"),
    ] {
        let sum = xherd_core::hash::content_sha256(&std::fs::read(artifact).unwrap());
        let version = runtime_requirement(plugin, tool).version.unwrap();

        let home = tempfile::tempdir().unwrap();
        // A leftover npm shim at ~/.local/bin/<cli> must be replaced.
        std::fs::create_dir_all(home.path().join(".local/bin")).unwrap();
        std::fs::write(home.path().join(".local/bin").join(tool), "shim").unwrap();
        let (ok, urls, stray) = run_install(plugin, tool, home.path(), artifact, &sum);
        assert!(ok, "{plugin} install failed");
        assert!(urls.contains(&version), "{plugin}: {urls}");
        assert!(stray.is_empty(), "{plugin}: left {stray:?}");
        assert!(home.path().join(placed).is_file(), "{plugin}: {placed}");
        let cli = std::fs::read_to_string(home.path().join(".local/bin").join(tool)).unwrap();
        assert_ne!(cli, "shim", "{plugin}: shim not replaced");
        let cache = home.path().join(".cache");
        let left: Vec<_> = std::fs::read_dir(&cache)
            .map(|d| d.map(|e| e.unwrap().file_name()).collect())
            .unwrap_or_default();
        assert!(
            left.is_empty(),
            "{plugin}: download left in .cache: {left:?}"
        );

        // Wrong digest: refused, nothing placed.
        let home = tempfile::tempdir().unwrap();
        let (ok, _, _) = run_install(plugin, tool, home.path(), artifact, &"00".repeat(32));
        assert!(!ok, "{plugin}: wrong digest accepted");
        assert!(
            !home.path().join(placed).exists(),
            "{plugin}: placed anyway"
        );
        // No digest for this CPU: refused too.
        let (ok, _, _) = run_install(plugin, tool, home.path(), artifact, "");
        assert!(!ok, "{plugin}: empty digest accepted");
    }
}

#[tokio::test]
async fn probe_returns_the_requirement_output() {
    let d = tempfile::tempdir().unwrap();
    let mut m = manifest("p");
    m["requires"] = json!([
        {"name": "ver", "check": ["./run", "ver"]},
        {"name": "gone", "check": ["./run", "fail"]}
    ]);
    let script = format!("{SCRIPT}\n");
    let script = script.replace("  junk)", "  ver) echo \"tool 1.2.3\" ;;\n  junk)");
    write_plugin(d.path(), "p", m, &script);
    let r = Registry::load(vec![], d.path());
    let p = r.get("p").unwrap();
    let c = ctx();
    let reqs = &p.manifest.requires;
    assert_eq!(probe(p, &c, &reqs[0]).await.unwrap().trim(), "tool 1.2.3");
    let e = probe(p, &c, &reqs[1]).await.unwrap_err();
    assert!(e.starts_with("bad token") && !e.contains("s3cr3t"), "{e}");
}
