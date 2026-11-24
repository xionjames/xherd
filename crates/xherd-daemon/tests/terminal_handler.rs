//! The `xherd-term://` handler script and its installer (R-707): the handler
//! is taken from a rendered install in a temporary `HOME`, then its
//! validation, decoding and the command per terminal are run in dry-run mode
//! with `sh` (and `bash` when found) against a temporary worktrees root, fake
//! terminals and a minimal `PATH`.

use std::io::Write;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use xherd_daemon::open_in::{HANDLER_SH, HANDLER_VERSION, handler_script};

const REPORT: &str = "http://127.0.0.1:9";

/// What the installer needs besides the shell's builtins.
const INSTALL_TOOLS: &[&str] = &[
    "awk", "cat", "chmod", "mkdir", "mv", "rm", "rmdir", "sed", "uname",
];

/// Every Linux terminal id, in the handler's order.
const LINUX: &[&str] = &[
    "ghostty",
    "konsole",
    "gnome-terminal",
    "kitty",
    "alacritty",
    "wezterm",
    "xfce4-terminal",
    "foot",
    "xterm",
    "xdg-terminal-exec",
    "x-terminal-emulator",
];

/// The shells the handler is run with, as argv prefixes: `sh` always,
/// `bash` and `busybox sh` when present.
fn shells() -> Vec<Vec<String>> {
    let found = |names: &[&str]| {
        names
            .iter()
            .find(|p| Path::new(p).exists())
            .map(|p| p.to_string())
    };
    let mut v = vec![sh()];
    if let Some(b) = found(&["/bin/bash", "/usr/bin/bash"]) {
        v.push(vec![b]);
    }
    if let Some(b) = found(&["/bin/busybox", "/usr/bin/busybox"]) {
        v.push(vec![b, "sh".into()]);
    }
    v
}

fn sh() -> Vec<String> {
    vec!["/bin/sh".into()]
}

fn which(tool: &str) -> PathBuf {
    for d in ["/usr/bin", "/bin", "/usr/local/bin"] {
        let p = Path::new(d).join(tool);
        if p.exists() {
            return p;
        }
    }
    panic!("{tool} not found");
}

struct Env {
    _tmp: tempfile::TempDir,
    base: PathBuf,
    home: PathBuf,
    root: PathBuf,
    /// The fake `PATH` the handler runs with.
    bin: PathBuf,
    /// Links to the installer's tools, before `bin` in its `PATH`.
    tools: PathBuf,
    /// The handler the install wrote.
    handler: PathBuf,
    /// The empty folder standing for `/Applications` (`XHERD_TERM_APPS_DIR`).
    apps: PathBuf,
}

/// Writes an executable script `name` running `body` in the fake `PATH`.
fn fake_tool(bin: &Path, name: &str, body: &str) {
    let p = bin.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A worktree as xherd makes it: `.git` file and `.xherd/` folder.
fn worktree(dir: &Path) {
    std::fs::create_dir_all(dir.join(".xherd")).unwrap();
    std::fs::write(dir.join(".git"), "gitdir: /elsewhere\n").unwrap();
}

fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let home = base.join("home");
    // A space in the root: the conf line and every check must cope.
    let root = base.join("x herd/worktrees");
    let bin = base.join("bin");
    let tools = base.join("tools");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&tools).unwrap();
    let apps = base.join("apps");
    std::fs::create_dir_all(&apps).unwrap();
    for t in ["awk", "sed", "uname"] {
        symlink(which(t), bin.join(t)).unwrap();
    }
    for t in INSTALL_TOOLS {
        symlink(which(t), tools.join(t)).unwrap();
    }
    worktree(&root.join("app/fix-login"));
    let handler = home.join(".local/bin/xherd-open-terminal");
    let e = Env {
        _tmp: tmp,
        base,
        home,
        root,
        bin,
        tools,
        handler,
        apps,
    };
    let o = install(&e, &sh(), "linux", &["install"], &[]);
    assert_eq!(o.code, 0, "install failed: {}", o.stderr);
    // A trailing slash in the conf: the handler must cope.
    std::fs::write(
        e.home.join(".config/xherd/terminal-handler.conf"),
        format!("version=1\nroot={}/\n", e.root.display()),
    )
    .unwrap();
    e
}

/// Runs the installer rendered for `e.root` as `sh -s <args>` with the script
/// on stdin (as `curl … | sh` does), in dry-run mode.
fn install(e: &Env, shell: &[String], os: &str, args: &[&str], extra: &[(&str, &str)]) -> Out {
    let script = handler_script(&e.root.to_string_lossy(), REPORT).unwrap();
    let path = format!("{}:{}", e.tools.display(), e.bin.display());
    let mut c = Command::new(&shell[0]);
    c.args(&shell[1..])
        .arg("-s")
        .args(args)
        .env_clear()
        .env("HOME", &e.home)
        .env("PATH", path)
        .env("XHERD_TERM_APPS_DIR", &e.apps)
        .env("XHERD_TERM_DRY_RUN", "1")
        .env("XHERD_TERM_OS", os)
        .current_dir(&e.base)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in extra {
        c.env(k, v);
    }
    let mut child = c.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let o = child.wait_with_output().unwrap();
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

/// Every file under `dir`, relative, with its content and mode.
fn files(dir: &Path) -> Vec<(String, Vec<u8>, u32)> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let p = entry.unwrap().path();
            let m = std::fs::symlink_metadata(&p).unwrap();
            if m.is_dir() {
                todo.push(p);
            } else {
                let rel = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
                out.push((
                    rel,
                    std::fs::read(&p).unwrap(),
                    m.permissions().mode() & 0o777,
                ));
            }
        }
    }
    out.sort();
    out
}

/// Percent-encodes every byte but unreserved ones and `/`.
fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

fn link(dir: &Path, terminal: Option<&str>) -> String {
    let mut u = format!("xherd-term://open?dir={}", enc(&dir.to_string_lossy()));
    if let Some(t) = terminal {
        u.push_str("&terminal=");
        u.push_str(t);
    }
    u
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_with(e: &Env, shell: &[String], os: &str, args: &[&str], extra: &[(&str, &str)]) -> Out {
    let mut c = Command::new(&shell[0]);
    c.args(&shell[1..])
        .arg(&e.handler)
        .args(args)
        .env_clear()
        .env("HOME", &e.home)
        .env("PATH", &e.bin)
        .env("XHERD_TERM_APPS_DIR", &e.apps)
        .env("XHERD_TERM_DRY_RUN", "1")
        .env("XHERD_TERM_OS", os)
        .current_dir(&e.base);
    for (k, v) in extra {
        c.env(k, v);
    }
    let o = c.output().unwrap();
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

/// Runs `open <url>` with every shell and checks they agree.
fn open(e: &Env, os: &str, url: &str, extra: &[(&str, &str)]) -> Out {
    let mut first: Option<Out> = None;
    for sh in shells() {
        let o = run_with(e, &sh, os, &["open", url], extra);
        if let Some(f) = &first {
            assert_eq!(
                (f.code, &f.stdout),
                (o.code, &o.stdout),
                "{sh:?} disagrees on {url}"
            );
        } else {
            first = Some(o);
        }
    }
    first.unwrap()
}

fn refused(e: &Env, url: &str) -> String {
    let o = open(e, "linux", url, &[]);
    assert_eq!(o.code, 1, "{url} was accepted: {}", o.stdout);
    assert!(o.stdout.is_empty(), "{url} printed {}", o.stdout);
    assert!(
        o.stderr.starts_with("xherd-open-terminal: "),
        "{}",
        o.stderr
    );
    o.stderr
}

fn argv(o: &Out) -> Vec<&str> {
    assert_eq!(o.code, 0, "refused: {}", o.stderr);
    o.stdout.lines().collect()
}

#[test]
fn opens_the_default_terminal_in_a_worktree() {
    let e = setup();
    fake_tool(&e.bin, "kitty", "exit 0");
    let dir = e.root.join("app/fix-login");
    let o = open(&e, "linux", &link(&dir, None), &[]);
    let want = format!("--directory\n{}\n", dir.display());
    assert_eq!(o.stdout, format!("kitty\n{want}"));
    // The bare URL form works too.
    let o = run_with(&e, &sh(), "linux", &[&link(&dir, None)], &[]);
    assert_eq!(argv(&o), ["kitty", "--directory", &dir.to_string_lossy()]);
}

#[test]
fn every_linux_terminal_has_its_command() {
    let e = setup();
    for t in LINUX {
        fake_tool(&e.bin, t, "exit 0");
    }
    let dir = e.root.join("app/fix-login");
    let d = dir.to_string_lossy().into_owned();
    let wd = format!("--working-directory={d}");
    let cases: &[(&str, Vec<&str>)] = &[
        ("ghostty", vec!["ghostty", &wd]),
        ("konsole", vec!["konsole", "--workdir", &d]),
        ("gnome-terminal", vec!["gnome-terminal", &wd]),
        ("kitty", vec!["kitty", "--directory", &d]),
        ("alacritty", vec!["alacritty", "--working-directory", &d]),
        ("wezterm", vec!["wezterm", "start", "--cwd", &d]),
        ("xfce4-terminal", vec!["xfce4-terminal", &wd]),
        ("foot", vec!["foot", &wd]),
        ("xterm", vec!["xterm"]),
        ("xdg-terminal-exec", vec!["xdg-terminal-exec"]),
        ("x-terminal-emulator", vec!["x-terminal-emulator"]),
    ];
    assert_eq!(cases.len(), LINUX.len());
    for (id, want) in cases {
        let o = open(&e, "linux", &link(&dir, Some(id)), &[]);
        assert_eq!(&argv(&o), want, "{id}");
    }
}

#[test]
fn every_mac_terminal_has_its_command() {
    let e = setup();
    for app in ["iTerm.app", "Ghostty.app"] {
        std::fs::create_dir_all(e.home.join("Applications").join(app)).unwrap();
    }
    let dir = e.root.join("app/fix-login");
    let d = dir.to_string_lossy().into_owned();
    let wd = format!("--working-directory={d}");
    let cases: &[(Option<&str>, Vec<&str>)] = &[
        (None, vec!["open", "-a", "Terminal", &d]),
        (Some("terminal"), vec!["open", "-a", "Terminal", &d]),
        (Some("iterm"), vec!["open", "-a", "iTerm", &d]),
        (
            Some("ghostty"),
            vec!["open", "-na", "Ghostty", "--args", &wd],
        ),
        // A Linux terminal on macOS: the default.
        (Some("kitty"), vec!["open", "-a", "Terminal", &d]),
    ];
    for (id, want) in cases {
        let o = open(&e, "mac", &link(&dir, *id), &[]);
        assert_eq!(&argv(&o), want, "{id:?}");
    }
    // Not installed: the default.
    std::fs::remove_dir_all(e.home.join("Applications/iTerm.app")).unwrap();
    let o = open(&e, "mac", &link(&dir, Some("iterm")), &[]);
    assert_eq!(argv(&o), ["open", "-a", "Terminal", &d]);
}

#[test]
fn unknown_or_missing_terminals_fall_back_to_the_default() {
    let e = setup();
    fake_tool(&e.bin, "konsole", "exit 0");
    let dir = e.root.join("app/fix-login");
    for t in ["nope", "iterm", "kitty", "konsole%3Bxterm", ""] {
        let o = open(&e, "linux", &link(&dir, Some(t)), &[]);
        assert_eq!(argv(&o)[0], "konsole", "{t}");
    }
}

#[test]
fn the_desktop_default_is_found() {
    let e = setup();
    for t in ["kitty", "konsole", "gnome-terminal", "foot"] {
        fake_tool(&e.bin, t, "exit 0");
    }
    let dir = e.root.join("app/fix-login");
    let first = |extra: &[(&str, &str)]| {
        let o = open(&e, "linux", &link(&dir, None), extra);
        argv(&o)[0].to_string()
    };
    // No desktop hint: the first installed in the list.
    assert_eq!(first(&[]), "konsole");
    // GNOME.
    fake_tool(&e.bin, "gsettings", "echo \"'foot'\"");
    assert_eq!(first(&[]), "foot");
    // KDE: TerminalApplication wins over TerminalService, in [General] only.
    let kde = e.home.join(".config/kdeglobals");
    std::fs::write(&kde, "[General]\nTerminalService=org.kde.konsole.desktop\n").unwrap();
    assert_eq!(first(&[]), "konsole");
    std::fs::write(
        &kde,
        "[Other]\nTerminalApplication=foot\n[General]\nTerminalService=org.kde.konsole.desktop\nTerminalApplication=/usr/bin/kitty -1\n",
    )
    .unwrap();
    assert_eq!(first(&[]), "kitty");
    // $TERMINAL first; an unknown or missing one is skipped.
    assert_eq!(first(&[("TERMINAL", "gnome-terminal")]), "gnome-terminal");
    assert_eq!(first(&[("TERMINAL", "alacritty")]), "kitty");
    assert_eq!(first(&[("TERMINAL", "rm -rf")]), "kitty");
}

#[test]
fn terminals_lists_the_installed_ones_default_first() {
    let e = setup();
    for t in ["kitty", "xterm", "foot"] {
        fake_tool(&e.bin, t, "exit 0");
    }
    let o = run_with(&e, &sh(), "linux", &["terminals"], &[("TERMINAL", "xterm")]);
    assert_eq!(o.stdout, "xterm\nkitty\nfoot\n");
    let o = run_with(&e, &sh(), "linux", &["terminals"], &[]);
    assert_eq!(o.stdout, "kitty\nfoot\nxterm\n");
    std::fs::create_dir_all(e.home.join("Applications/Ghostty.app")).unwrap();
    let o = run_with(&e, &sh(), "mac", &["terminals"], &[]);
    assert_eq!(o.stdout, "terminal\nghostty\n");
    // The apps folder stands for `/Applications`: only what is in it counts.
    std::fs::remove_dir_all(e.home.join("Applications")).unwrap();
    let o = run_with(&e, &sh(), "mac", &["terminals"], &[]);
    assert_eq!(o.stdout, "terminal\n");
    std::fs::create_dir_all(e.apps.join("Ghostty.app")).unwrap();
    let o = run_with(&e, &sh(), "mac", &["terminals"], &[]);
    assert_eq!(o.stdout, "terminal\nghostty\n");
}

#[test]
fn decoding_handles_spaces_slashes_and_utf8() {
    let e = setup();
    fake_tool(&e.bin, "kitty", "exit 0");
    let dir = e.root.join("my app/agent-ñandú");
    worktree(&dir);
    let d = dir.to_string_lossy().into_owned();
    let o = open(&e, "linux", &link(&dir, None), &[]);
    assert_eq!(argv(&o), ["kitty", "--directory", &d]);
    // `%2F` for the slashes, lower-case escapes.
    let url = format!(
        "xherd-term://open?terminal=kitty&dir={}",
        enc(&d).replace('/', "%2f")
    );
    let o = open(&e, "linux", &url, &[]);
    assert_eq!(argv(&o), ["kitty", "--directory", &d]);
    // Unknown keys are ignored; the `open/` form is accepted.
    let url = format!("xherd-term://open/?x=1&dir={}", enc(&d));
    let o = open(&e, "linux", &url, &[]);
    assert_eq!(argv(&o), ["kitty", "--directory", &d]);
}

#[test]
fn shell_metacharacters_are_never_run() {
    let e = setup();
    fake_tool(&e.bin, "kitty", "exit 0");
    fake_tool(&e.bin, "touch", "echo ran >> \"$HOME/../ran\"");
    // A real worktree with a hostile name opens, the name passed as data.
    let evil = "$(touch pwned);`touch pwned`'\"&|<>*?";
    let dir = e.root.join("app").join(evil);
    worktree(&dir);
    let d = dir.to_string_lossy().into_owned();
    let o = open(&e, "linux", &link(&dir, None), &[]);
    assert_eq!(argv(&o), ["kitty", "--directory", &d]);
    let o = open(&e, "linux", &link(&dir, Some("kitty")), &[]);
    assert_eq!(argv(&o), ["kitty", "--directory", &d]);
    // The same in a folder that does not exist, and raw (unencoded) ones.
    refused(&e, &link(&e.root.join("app/$(touch pwned)"), None));
    for raw in [
        "`touch%20pwned`",
        "a;touch pwned",
        "$(touch)\"",
        "a|b",
        "x\ny",
    ] {
        let url = format!(
            "xherd-term://open?dir={}/app/{raw}",
            enc(&e.root.to_string_lossy())
        );
        refused(&e, &url);
    }
    assert!(!e.base.join("ran").exists(), "a command ran");
    assert!(!e.base.join("pwned").exists());
}

#[test]
fn only_an_agent_worktree_itself_is_accepted() {
    let e = setup();
    fake_tool(&e.bin, "kitty", "exit 0");
    let root = e.root.clone();
    let r = root.to_string_lossy().into_owned();
    let er = enc(&r);

    // Outside the worktree shape.
    let outside = e.base.join("elsewhere/agent");
    worktree(&outside);
    std::fs::create_dir_all(root.join("app/fix-login/src")).unwrap();
    worktree(&root.join("app/fix-login/sub/inner"));
    std::fs::create_dir_all(root.join("app/no-git/.xherd")).unwrap();
    std::fs::create_dir_all(root.join("app/no-xherd")).unwrap();
    std::fs::write(root.join("app/no-xherd/.git"), "gitdir: x\n").unwrap();
    std::fs::create_dir_all(root.join("app/clone/.git")).unwrap();
    std::fs::create_dir_all(root.join("app/clone/.xherd")).unwrap();
    std::fs::create_dir_all(root.join("app/xherd-link")).unwrap();
    std::fs::write(root.join("app/xherd-link/.git"), "gitdir: x\n").unwrap();
    symlink(outside.join(".xherd"), root.join("app/xherd-link/.xherd")).unwrap();
    worktree(&root.join("-app/agent"));
    // A symlinked worktree, and a repo folder that is a symlink out of the root.
    symlink(&outside, root.join("app/linked")).unwrap();
    symlink(e.base.join("elsewhere"), root.join("evil")).unwrap();

    let cases = [
        format!("{er}/app/fix-login/src"),
        format!("{er}/app/fix-login/sub/inner"),
        format!("{er}/app/fix-login/"),
        er.clone(),
        format!("{er}/"),
        format!("{er}/app"),
        format!("{er}/app/no-git"),
        format!("{er}/app/no-xherd"),
        format!("{er}/app/clone"),
        format!("{er}/app/xherd-link"),
        format!("{er}/app/missing"),
        format!("{er}/-app/agent"),
        format!("{er}/app/linked"),
        format!("{er}/evil/agent"),
        format!("{er}/app/../app/fix-login"),
        format!("{er}/app/./fix-login"),
        format!("{er}/../worktrees/app/fix-login"),
        format!("{er}/%2E%2E/worktrees/app/fix-login"),
        enc(&outside.to_string_lossy()),
        "app/fix-login".into(),
        String::new(),
    ];
    for dir in &cases {
        refused(&e, &format!("xherd-term://open?dir={dir}"));
    }
    // The right one, for comparison.
    let ok = open(
        &e,
        "linux",
        &format!("xherd-term://open?dir={er}/app/fix-login"),
        &[],
    );
    assert_eq!(ok.code, 0, "{}", ok.stderr);
}

#[test]
fn malformed_links_are_refused() {
    let e = setup();
    fake_tool(&e.bin, "kitty", "exit 0");
    let d = enc(&e.root.join("app/fix-login").to_string_lossy());
    for url in [
        format!("https://open?dir={d}"),
        format!("xherd-term://other?dir={d}"),
        format!("XHERD-TERM://open?dir={d}"),
        "xherd-term://open?terminal=kitty".into(),
        format!("xherd-term://open?dir={d}&dir={d}"),
        format!("xherd-term://open?dir={d}&terminal=kitty&terminal=xterm"),
        format!("xherd-term://open?dir={d}%zz"),
        format!("xherd-term://open?dir={d}%2"),
        format!("xherd-term://open?dir={d}%0A"),
        format!("xherd-term://open?dir={d}%00"),
        format!("xherd-term://open?dir={d}%7F"),
        format!("xherd-term://open?dir={d}#x"),
        format!("xherd-term://open?dir={d} "),
        format!("xherd-term://open?dir={d}&terminal=%0A"),
        format!("xherd-term://open?dir={}", "a".repeat(9000)),
    ] {
        refused(&e, &url);
    }
    // Wrong usage.
    let o = run_with(&e, &sh(), "linux", &[], &[]);
    assert_eq!(o.code, 1);
    let o = run_with(&e, &sh(), "linux", &["open"], &[]);
    assert_eq!(o.code, 1);
}

#[test]
fn a_missing_or_broken_setup_refuses() {
    let e = setup();
    fake_tool(&e.bin, "kitty", "exit 0");
    let url = link(&e.root.join("app/fix-login"), None);
    let conf = e.home.join(".config/xherd/terminal-handler.conf");
    std::fs::write(&conf, "version=1\n").unwrap();
    assert!(refused(&e, &url).contains("not set up"));
    std::fs::write(&conf, "root=relative/path\n").unwrap();
    assert!(refused(&e, &url).contains("not set up"));
    std::fs::write(&conf, format!("root={}/gone\n", e.base.display())).unwrap();
    assert!(refused(&e, &url).contains("missing"));
    std::fs::remove_file(&conf).unwrap();
    assert!(refused(&e, &url).contains("not set up"));
    // No terminal at all.
    std::fs::write(&conf, format!("root={}\n", e.root.display())).unwrap();
    std::fs::remove_file(e.bin.join("kitty")).unwrap();
    assert!(refused(&e, &url).contains("no terminal"));
}

#[test]
fn refusals_never_echo_the_link() {
    let e = setup();
    let url = "xherd-term://open?dir=/secret-marker/$(x)";
    let msg = refused(&e, url);
    assert!(!msg.contains("secret-marker"), "{msg}");
}

#[test]
fn the_handler_comes_from_the_install() {
    let e = setup();
    assert_eq!(std::fs::read_to_string(&e.handler).unwrap(), HANDLER_SH);
    let mode = std::fs::metadata(&e.handler).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
}

#[test]
fn install_on_linux_writes_and_registers() {
    let e = setup();
    fake_tool(&e.bin, "kitty", "exit 0");
    fake_tool(&e.bin, "foot", "exit 0");
    for sh in shells() {
        let o = install(&e, &sh, "linux", &[], &[]);
        assert_eq!(o.code, 0, "{sh:?}: {}", o.stderr);
        let apps = e.home.join(".local/share/applications");
        let root = e.root.display();
        assert_eq!(
            o.stdout.lines().collect::<Vec<_>>(),
            [
                "xdg-mime default xherd-term.desktop x-scheme-handler/xherd-term".to_string(),
                format!("update-desktop-database {}", apps.display()),
                format!(
                    "curl -fsS -m 10 -o /dev/null -H Content-Type: application/json -d \
                     {{\"version\":{HANDLER_VERSION},\"xherd\":\"{}\",\"os\":\"linux\",\
                     \"terminals\":[\"kitty\",\"foot\"],\"root\":\"{root}\"}} \
                     {REPORT}/api/v1/open-in/terminal-handler",
                    xherd_core::BUILD_VERSION
                ),
            ],
            "{sh:?}"
        );
        let conf = e.home.join(".config/xherd/terminal-handler.conf");
        assert_eq!(
            std::fs::read_to_string(&conf).unwrap(),
            format!("root={root}\nversion={HANDLER_VERSION}\n")
        );
        assert_eq!(
            std::fs::metadata(&conf).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let desktop = std::fs::read_to_string(apps.join("xherd-term.desktop")).unwrap();
        assert!(desktop.starts_with("[Desktop Entry]\n"), "{desktop}");
        assert!(
            desktop.contains(&format!("\nExec=\"{}\" open %u\n", e.handler.display())),
            "{desktop}"
        );
        assert!(desktop.contains("\nNoDisplay=true\n"));
        assert!(desktop.contains("\nMimeType=x-scheme-handler/xherd-term;\n"));
    }
    // XDG_DATA_HOME moves the desktop entry.
    let data = e.base.join("data");
    let o = install(
        &e,
        &sh(),
        "linux",
        &[],
        &[("XDG_DATA_HOME", &data.to_string_lossy())],
    );
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(data.join("applications/xherd-term.desktop").is_file());
}

#[test]
fn installing_twice_changes_nothing_and_replaces_an_older_install() {
    let e = setup();
    let first = install(&e, &sh(), "linux", &[], &[]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let before = files(&e.home);
    let again = install(&e, &sh(), "linux", &[], &[]);
    assert_eq!(again.code, 0, "{}", again.stderr);
    assert_eq!(files(&e.home), before);
    assert_eq!(again.stdout, first.stdout);
    // An older install: every file is replaced.
    let desktop = e.home.join(".local/share/applications/xherd-term.desktop");
    let conf = e.home.join(".config/xherd/terminal-handler.conf");
    std::fs::write(&e.handler, "#!/bin/sh\necho old\n").unwrap();
    std::fs::write(&desktop, "[Desktop Entry]\nExec=old\n").unwrap();
    std::fs::write(&conf, "root=/old\nversion=0\n").unwrap();
    let o = install(&e, &sh(), "linux", &[], &[]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert_eq!(files(&e.home), before);
    // No temporary files left behind.
    assert!(
        before.iter().all(|(p, _, _)| !p.contains(".tmp.")),
        "{before:?}"
    );
}

#[test]
fn uninstall_on_linux_leaves_nothing() {
    let e = setup();
    let mimeapps = e.home.join(".config/mimeapps.list");
    std::fs::write(
        &mimeapps,
        "[Default Applications]\nx-scheme-handler/xherd-term=xherd-term.desktop\ntext/plain=kate.desktop\n",
    )
    .unwrap();
    for sh in shells() {
        let o = install(&e, &sh, "linux", &["uninstall"], &[]);
        assert_eq!(o.code, 0, "{sh:?}: {}", o.stderr);
        assert_eq!(
            o.stdout.lines().collect::<Vec<_>>(),
            [
                format!(
                    "update-desktop-database {}",
                    e.home.join(".local/share/applications").display()
                ),
                format!(
                    "curl -fsS -m 10 -o /dev/null -H Content-Type: application/json -d \
                     {{\"installed\":false}} {REPORT}/api/v1/open-in/terminal-handler"
                ),
            ]
        );
        let left: Vec<_> = files(&e.home).into_iter().map(|f| f.0).collect();
        assert_eq!(left, [".config/mimeapps.list"], "{sh:?}");
        assert_eq!(
            std::fs::read_to_string(&mimeapps).unwrap(),
            "[Default Applications]\ntext/plain=kate.desktop\n"
        );
    }
}

#[test]
fn install_and_uninstall_on_macos_dry_run() {
    let e = setup();
    std::fs::create_dir_all(e.home.join("Applications/iTerm.app")).unwrap();
    let support = e.home.join("Library/Application Support/xherd");
    let handler = support.join("xherd-open-terminal");
    let app = support.join("XherdTerminal.app");
    let plist = app.join("Contents/Info.plist");
    let pb = "/usr/libexec/PlistBuddy";
    let ls = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    let id = "io.github.xionjames.xherd.terminal";
    for sh in shells() {
        let o = install(&e, &sh, "mac", &[], &[]);
        assert_eq!(o.code, 0, "{sh:?}: {}", o.stderr);
        assert_eq!(std::fs::read_to_string(&handler).unwrap(), HANDLER_SH);
        assert_eq!(
            o.stdout.lines().collect::<Vec<_>>(),
            [
                format!(
                    "osacompile -o {} -e on open location u -e do shell script quoted form of \"{}\" \
                     & \" open \" & quoted form of u & \" >/dev/null 2>&1 &\" -e end open location",
                    app.display(),
                    handler.display()
                ),
                format!("{pb} -c Set :CFBundleIdentifier {id} {}", plist.display()),
                format!("{pb} -c Add :LSUIElement bool true {}", plist.display()),
                format!(
                    "{pb} -c Add :CFBundleURLTypes array -c Add :CFBundleURLTypes:0 dict \
                     -c Add :CFBundleURLTypes:0:CFBundleURLName string {id} \
                     -c Add :CFBundleURLTypes:0:CFBundleURLSchemes array \
                     -c Add :CFBundleURLTypes:0:CFBundleURLSchemes:0 string xherd-term {}",
                    plist.display()
                ),
                format!("{ls} -f {}", app.display()),
                format!(
                    "curl -fsS -m 10 -o /dev/null -H Content-Type: application/json -d \
                     {{\"version\":{HANDLER_VERSION},\"xherd\":\"{}\",\"os\":\"mac\",\
                     \"terminals\":[\"terminal\",\"iterm\"],\"root\":\"{}\"}} \
                     {REPORT}/api/v1/open-in/terminal-handler",
                    xherd_core::BUILD_VERSION,
                    e.root.display()
                ),
            ],
            "{sh:?}"
        );
        let o = install(&e, &sh, "mac", &["uninstall"], &[]);
        assert_eq!(o.code, 0, "{sh:?}: {}", o.stderr);
        assert_eq!(
            o.stdout.lines().next(),
            Some(&*format!("{ls} -u {}", app.display()))
        );
        assert!(!support.exists(), "{sh:?}");
        assert!(!e.home.join(".config/xherd/terminal-handler.conf").exists());
    }
}

#[test]
fn a_real_run_registers_and_reports_or_warns() {
    let e = setup();
    let log = e.base.join("log");
    let l = log.display();
    fake_tool(&e.bin, "kitty", "exit 0");
    fake_tool(
        &e.bin,
        "xdg-mime",
        &format!("echo \"xdg-mime $*\" >> '{l}'"),
    );
    fake_tool(
        &e.bin,
        "update-desktop-database",
        &format!("echo \"udd $*\" >> '{l}'"),
    );
    // No curl or wget: installed, with a warning, exit 0.
    let real = [("XHERD_TERM_DRY_RUN", "")];
    let o = install(&e, &sh(), "linux", &[], &real);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stderr.contains("xherd was not told"), "{}", o.stderr);
    let apps = e.home.join(".local/share/applications");
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        format!(
            "xdg-mime default xherd-term.desktop x-scheme-handler/xherd-term\nudd {}\n",
            apps.display()
        )
    );
    // A curl that fails: the same warning.
    fake_tool(&e.bin, "curl", "exit 7");
    let o = install(&e, &sh(), "linux", &[], &real);
    assert_eq!(o.code, 0);
    assert!(o.stderr.contains("xherd was not told"), "{}", o.stderr);
    // A curl that works: no warning, the body as one argument.
    fake_tool(
        &e.bin,
        "curl",
        &format!("for a in \"$@\"; do echo \"[$a]\"; done >> '{l}.curl'"),
    );
    let o = install(&e, &sh(), "linux", &["uninstall"], &real);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(!o.stderr.contains("not told"), "{}", o.stderr);
    let args = std::fs::read_to_string(format!("{l}.curl")).unwrap();
    assert!(args.contains("\n[{\"installed\":false}]\n"), "{args}");
    // wget when there is no curl.
    std::fs::remove_file(e.bin.join("curl")).unwrap();
    fake_tool(
        &e.bin,
        "wget",
        &format!("for a in \"$@\"; do echo \"[$a]\"; done >> '{l}.wget'"),
    );
    let o = install(&e, &sh(), "linux", &[], &real);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let args = std::fs::read_to_string(format!("{l}.wget")).unwrap();
    assert!(args.contains("[--post-data={\"version\":"), "{args}");
    assert!(args.contains(&format!("[{REPORT}/api/v1/open-in/terminal-handler]")));
}

#[test]
fn install_refuses_a_home_it_cannot_quote_and_bad_usage() {
    let e = setup();
    let odd = e.base.join("we\"ird");
    std::fs::create_dir_all(&odd).unwrap();
    let o = install(&e, &sh(), "linux", &[], &[("HOME", &odd.to_string_lossy())]);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("cannot be registered"), "{}", o.stderr);
    assert!(files(&odd).is_empty());
    let o = install(&e, &sh(), "linux", &["remove"], &[]);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("usage"), "{}", o.stderr);
}
