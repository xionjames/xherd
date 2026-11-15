//! Sets `XHERD_BUILD_VERSION`, the version shown to people: the git tag of a
//! release build (`0.2.0`), `<tag>+<commits>.<hash>` between releases
//! (`0.2.0+3.abc1234`, SemVer build metadata), `<cargo version>+<hash>` before
//! the first tag, `+dirty` for uncommitted changes. `XHERD_VERSION` in the
//! environment wins (builds without `.git`, such as the Docker image); with
//! neither, `dev`. The rules are in `version.rs`, tested from `src/lib.rs`.

use std::process::Command;

include!("version.rs");

fn describe() -> Option<String> {
    let out = Command::new("git")
        .args(["describe", "--tags", "--long", "--always", "--dirty"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    let cargo = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    println!("cargo:rerun-if-env-changed=XHERD_VERSION");
    println!("cargo:rerun-if-changed=version.rs");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");
    let env = std::env::var("XHERD_VERSION").ok();
    let v = pick(env.as_deref(), describe().as_deref(), &cargo);
    println!("cargo:rustc-env=XHERD_BUILD_VERSION={v}");
}
