# AGENTS.md

Guidance for coding agents (Claude Code, Codex, opencode…) working in this repository. **Keep this file current**: when you add a crate, a command, a convention or a gotcha, update it in the same change.

## What this is

xherd: a daemon (Rust) that runs, watches and steers many coding agents, each in its own git worktree and tmux session, with approval gates. Read before changing behaviour:

- `doc/spec.md` — what (requirements `R-xxx`)
- `doc/design.md` — how (crates, architecture, milestones §18, decisions §19)
- `doc/xap.md` — agent protocol (`xherd hook|report|gate|task|commit …`)

Decisions in `doc/design.md` §19 are settled; do not re-open them in code. Cite requirement ids (`R-009`) in doc comments where code implements one.

## Status

Milestone **M0 (skeleton)** done on branch `m0-skeleton`. Next: **M1** (one agent end to end: `xherd new`, worktree + `.xherd/`, Claude Code runtime plugin, XAP gates, hash approvals). Agents cannot be created from the CLI/API yet; tests seed them directly.

## Layout

| Path | Contents |
|---|---|
| `crates/xherd-core` | Domain: `Status` + lifecycle table (`status::next_status`), `Agent` (`fire(Trigger)`), `Report` + `report::apply_report` (sequence rules R-009, no revival R-008), flow types (`Gate`, `Task`, `Commit`, `Role`), `Paths`, `Config`/`Secrets` (schema migrations, unknown keys kept), `queue` (`hook-queue.jsonl`, sync, file-locked), `exec::run` (the only way to run external commands). Pure and heavily tested (incl. proptest). |
| `crates/xherd-store` | SQLite (WAL, one writer thread, read-only pool), forward-only migrations in `schema.rs` (append only), `VACUUM INTO` copy before migrating. |
| `crates/xherd-daemon` | `Daemon::start` (config, user token, store, plugins, supervisor, queue replay, reconcile, scheduler); `supervisor` (slots, start queue, port blocks); `actor` (one per agent: `Msg::{Report, User, Tick, WorkDone}`); `driver` (`Driver` trait, `TerminalDriver` on tmux, `FakeDriver` for tests); `scheduler` (all periodic jobs); `support` (clock, change hub, tokens). |
| `crates/xherd-api` | axum router: `/api/v1/{healthz,state,agents,agents/{id},agents/{id}/{pause,resume,cancel,send},hooks}`; `User` and `AgentAuth` extractors. |
| `crates/xherd-plugin` | `xherd-plugin.json` manifests, `Registry` (built-ins overridden by external, broken listed), settings/secrets resolution, command protocol (`run_command`, `run_typed`, `run_hook`, `run_checks`). |
| `crates/xherd-term` | `Tmux` (exact `=name` targets, optional `-L` socket), `ScreenRules`. PTY bridge comes with the UI (M2). |
| `crates/xherd-git` | `Git`: worktrees (`--no-track`), diffs, commit files, bundles, `info/exclude`. |
| `crates/xherd-cli` | Binary `xherd`: `daemon`, `status`, `ls`, `pause|resume|cancel|send <agent>`, `hook <event>`, `report …`. |

## Running it

```sh
XHERD_HOME=/tmp/xh cargo run -p xherd-cli -- daemon     # JSON logs on stderr; XHERD_LOG=debug for more
XHERD_HOME=/tmp/xh cargo run -p xherd-cli -- status
```

Data root: `XHERD_HOME`, else `$XDG_DATA_HOME/xherd`, else `~/.local/share/xherd`. `config.json` there holds the port (4740), `tmuxSocket` and the generated user token used by the CLI.

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

## Gotchas

- `xherd hook` is handled in `main` **before** clap, inside `catch_unwind` with a silent panic hook, so bad arguments or panics still exit 0 with no output. Keep it that way.
- Sequence numbers from the CLI are `max(last + 1, now in µs)` (`.xherd/seq`), so counter-based and time-based reports from the same agent stay ordered.
- Hooks without a token, or with the daemon unreachable / 5xx, are appended to `hook-queue.jsonl`; 4xx answers are not queued (a retry would not help). The daemon replays the queue at start and every minute.
- `rusqlite` is pinned to the version `rusqlite_migration` uses (both link `sqlite3`); bump them together.
- `ureq` 3 treats HTTP errors as `Err` unless `http_status_as_error(false)`; the CLI sets it.
- In axum handlers, read the peer address through the `ConnectInfo` extractor (not `extensions.get`) so `MockConnectInfo` works in tests.
- Supervisor messages carry `oneshot` replies straight to the caller; the supervisor never awaits an actor.

## Commits

One commit per working piece on the milestone branch; tests and clippy pass at each commit.
