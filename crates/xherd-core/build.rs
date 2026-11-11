//! Sets `XHERD_BUILD_VERSION`, the version shown to people: the git tag of a
//! release build (`0.2.0`), `<tag>+<commits>.<hash>` between releases
//! (`0.2.0+3.abc1234`, SemVer build metadata), `<cargo version>+<hash>` before
//! the first tag, `+dirty` for uncommitted changes. `XHERD_VERSION` in the
//! environment wins (builds without `.git`, such as the Docker image); with
//! neither, the Cargo version.

use std::process::Command;

fn describe() -> Option<String> {
    let out = Command::new("git")
        .args(["describe", "--tags", "--long", "--always", "--dirty"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `v0.2.0-3-gabc1234[-dirty]` or `abc1234[-dirty]` → display version.
fn display(raw: &str, cargo: &str) -> String {
    let (raw, dirty) = match raw.strip_suffix("-dirty") {
        Some(r) => (r, true),
        None => (raw, false),
    };
    let mut parts = raw.rsplitn(3, '-');
    let (hash, n, tag) = (parts.next(), parts.next(), parts.next());
    let mut v = match (tag, n.and_then(|n| n.parse::<u32>().ok()), hash) {
        (Some(tag), Some(0), _) => tag.trim_start_matches('v').to_string(),
        (Some(tag), Some(n), Some(h)) => {
            format!(
                "{}+{n}.{}",
                tag.trim_start_matches('v'),
                h.trim_start_matches('g')
            )
        }
        _ => format!("{cargo}+{raw}"),
    };
    if dirty {
        v.push_str(if v.contains('+') { ".dirty" } else { "+dirty" });
    }
    v
}

fn main() {
    let cargo = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    println!("cargo:rerun-if-env-changed=XHERD_VERSION");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");
    let v = match std::env::var("XHERD_VERSION") {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => describe().map_or(cargo.clone(), |d| display(&d, &cargo)),
    };
    println!("cargo:rustc-env=XHERD_BUILD_VERSION={v}");
}
