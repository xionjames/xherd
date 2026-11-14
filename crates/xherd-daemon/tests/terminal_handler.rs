//! The `xherd-term://` handler script (R-707): validation, decoding and the
//! command per terminal, run in dry-run mode with `sh` (and `bash` when found)
//! against a temporary worktrees root, fake terminals and a minimal `PATH`.

use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;

const HANDLER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/terminal-handler.sh");

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
    bin: PathBuf,
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
    std::fs::create_dir_all(home.join(".config/xherd")).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        home.join(".config/xherd/terminal-handler.conf"),
        format!("version=1\nroot={}/\n", root.display()),
    )
    .unwrap();
    for t in ["awk", "sed", "uname"] {
        symlink(which(t), bin.join(t)).unwrap();
    }
    worktree(&root.join("app/fix-login"));
    Env {
        _tmp: tmp,
        base,
        home,
        root,
        bin,
    }
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
        .arg(HANDLER)
        .args(args)
        .env_clear()
        .env("HOME", &e.home)
        .env("PATH", &e.bin)
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
