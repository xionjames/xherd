# AGENTS.md

Guidance for coding agents (Claude Code, Codex, opencode…) working in this repository. **Keep this file current**: when you add a crate, a command, a convention or a gotcha, update it in the same change.

## What this is

xherd: a daemon (Rust) that runs, watches and steers many coding agents, each in its own git worktree and tmux session, with approval gates. Read before changing behaviour:

- `doc/spec.md` — what (requirements `R-xxx`)
- `doc/design.md` — how (crates, architecture, milestones §18, decisions §19)
- `doc/xap.md` — agent protocol (`xherd hook|report|gate|task|commit …`)

Decisions in `doc/design.md` §19 are settled; do not re-open them in code. Cite requirement ids (`R-009`) in doc comments where code implements one.

## Status

Milestone **M0 (skeleton)** in progress on branch `m0-skeleton`. Nothing user-facing yet.

## Layout

| Path | Contents |
|---|---|
| `crates/xherd-core` | Domain: `Status` + lifecycle table (`next_status`), `Agent`, `Report` + `apply_report` (sequence rules), flow types (`Gate`, `Task`, `Commit`, `Role`), `Paths`, `Config`/`Secrets`, `exec::run` (the only way to run external commands). Pure and heavily tested. |
| `crates/xherd-store` | SQLite schema, migrations, writer task. |
| `crates/xherd-daemon` | Supervisor, agent actors, scheduler. |
| `crates/xherd-api` | axum routes and auth. |
| `crates/xherd-plugin` | Plugin manifests, settings/secrets, command protocol. |
| `crates/xherd-term` | tmux wrapper, screen rules. |
| `crates/xherd-git` | git wrappers. |
| `crates/xherd-cli` | The `xherd` binary. |

## Build and test

Rust is installed through Homebrew's keg-only `rustup`; add it to `PATH` if `cargo` is missing:

```sh
export PATH=/opt/homebrew/opt/rustup/bin:$PATH
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Integration tests need `git` and `tmux` (≥ 3.3) on `PATH`; tmux tests use a private socket (`-L xherd-test-…`) and never touch your sessions.

## Conventions

- Edition 2024, workspace lints: `unsafe_code = "forbid"`, clippy `all` = warn (CI denies warnings).
- External commands only through `xherd_core::exec::run` (argv, no shell, timeout + process-group kill, env allowlist, redaction).
- State changes to an agent only inside its actor, through pure functions in `xherd-core`; slow work returns as `WorkDone` tagged with the agent's `generation`.
- `xherd hook` / `xherd report` must never print to stdout or exit non-zero, and must not start tokio (fast path, R-012, R-901).
- JSON on the wire and in files is camelCase; enum values are kebab-case.
- Files the daemon writes go through `paths::write_atomic`; secrets are mode 0600.
- Minimal code: no abstraction for a single use; flag unrelated problems instead of fixing them silently.
- Clean room: never copy or name proprietary tools, prompts, texts or design tokens from other products.

## Commits

One commit per working piece on the milestone branch; tests and clippy pass at each commit.
