# Development

For people changing xherd itself. Read [Architecture](architecture.md) first. `AGENTS.md` at the repository root is the working guide for coding agents and records the current status and gotchas; keep it current when you change behaviour.

## Prerequisites

- Rust (edition 2024). On macOS with Homebrew's keg-only rustup: `export PATH=/opt/homebrew/opt/rustup/bin:$PATH`.
- Node and pnpm for the UI. Cargo does not build it.
- `git` and `tmux` ≥ 3.3 (integration tests start real ones).
- Docker with Compose, only to work on the container setup.

## Repository layout

| Path | Contents |
|---|---|
| `crates/` | The Rust workspace (see [Architecture](architecture.md#crates)) |
| `ui/` | Web UI: Svelte 5, Vite, TypeScript, pnpm |
| `packs/xherd/` | The built-in flow pack: `xherd-flow.yaml`, `FORMAT.md` (document formats), `commands/*.md` (agent prompts) |
| `plugins/` | Built-in plugin manifests: `claude-code`, `github`, `sdkman`, `nvm`, `uv`, `go`, `rustup` |
| `deploy/docker`, `deploy/compose` | Dockerfile and the compose file |
| `doc/` | This documentation |

## Build, run, test

```sh
# daemon with a throwaway data root
XHERD_HOME=/tmp/xh cargo run -p xherd-cli -- daemon     # JSON logs on stderr; XHERD_LOG=debug for more
XHERD_HOME=/tmp/xh cargo run -p xherd-cli -- status
XHERD_HOME=/tmp/xh cargo run -p xherd-cli -- install    # /xherd:* commands into ${CLAUDE_CONFIG_DIR:-~/.claude}/commands/xherd/

# checks to pass before every commit
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check

# UI
cd ui && pnpm install && pnpm build     # ui/dist, served by the daemon
cd ui && pnpm dev                       # Vite dev server, proxies /api to XHERD_URL (default 127.0.0.1:4740)
cd ui && pnpm check                     # svelte-check
```

Debug builds serve `ui/dist` from disk; release builds embed it at compile time, so build the UI before `cargo build --release`.

xherd marks each new worktree trusted in Claude Code's config, so no trust step is needed. Claude Code must be logged in once (`claude`, then `/login`).

### Demo data for screenshots

`scripts/demo.sh [dir]` runs a throwaway daemon on <http://127.0.0.1:4741/> with fake agents (`scripts/demo-seed.py`): two waiting at a gate, one watching a PR, one paused, one done. It uses its own data root, workspace and `HOME` plus stub `claude`/`gh` commands, so nothing of yours is touched. It needs a built daemon and `ui/dist`. Agents are never "running" (that needs a live tmux session). Warnings about "restored worktree not prepared" are expected: the fake worktrees are plain folders.

### Tests

- Unit and property tests in `xherd-core` and `xherd-flow` cover the pure logic (lifecycle table, report ordering, XAP validation, hashing, tripwires).
- Integration tests in `xherd-daemon/tests/` and `xherd-cli/tests/` use real `git` and `tmux`. tmux tests use a private socket (`-L xherd-test-…`) and never touch your sessions.
- Tests must never write to the real `$HOME` or `~/xherd`: set `workspace` in `config.json`, point the Claude Code trust file at a fixture, pass your own `env` closure, and use a fake `gh` script (`pluginSettings.github.gh`). See `tests/create.rs`, `tests/checks.rs` and `tests/xap.rs` for the setups.
- `prompt_calls_parse` parses every `xherd …` call in the pack prompts with clap; `every_template_has_a_prompt` checks each manifest command has a prompt file.

## Conventions

- Workspace lints: `unsafe_code = "forbid"`; clippy `all` is a warning. CI (`.github/workflows/ci.yml`) runs fmt, clippy with `-D warnings`, the tests, the UI check/test/build and an image build on pushes to main and PRs into main; run the same checks before pushing.
- External commands only through `xherd_core::exec::run`. Exceptions: the interactive tmux client of the browser terminal (`xherd_term::pty`), processes that replace the CLI (`toolchain exec`, `attach`, `shell`, `init` → daemon), and long-lived helpers (`socat`, apps launched by `open`/`companion`).
- Agent state changes only inside the agent's actor, through pure functions in `xherd-core`. Slow work returns as `WorkDone` tagged with the generation.
- `xherd hook` and `xherd report` must never print to stdout or exit non-zero, and must not start tokio. `hook` is handled before clap, inside `catch_unwind`.
- JSON on the wire and in files is camelCase; enum values are kebab-case.
- Files the daemon writes go through `paths::write_atomic`; secrets are mode 0600.
- Pack manifests and `ConfigPatch` use `deny_unknown_fields`.
- Keep code minimal: no abstraction for a single use. Cite a requirement id in doc comments where code implements one.
- UI: tokens and component classes live in `ui/src/app.css`. `--gate` (coral) means "needs a human" and nothing else. Status is never colour alone. Agent-written Markdown goes through `lib/markdown.ts` (marked + DOMPurify), never raw. UI panels key effects on `const id = $derived(agent.id)`, not `agent.id`.
- Dependencies: `rusqlite` is pinned to the version `rusqlite_migration` uses; bump both together.

## Common changes

**Add a XAP command** — add the wire type in `xherd-core::xap`, validation in `xherd_flow::xap::apply`, observation in `Actor::on_xap`, the CLI subcommand in `xherd-cli/src/xap.rs`, the prompt text where agents learn it, and document it in [xap.md](xap.md). Calls are de-duplicated by call id.

**Change a flow or prompt** — edit `packs/xherd/`. A new file must be added to `xherd_flow::install::BUILTIN_FILES` (a test compares the folder to the list). Prompts teach plain `xherd <call>` on one line inside backticks, never through a variable.

**Add a setting** — add the field to `Config` (`xherd-core`), to `ConfigPatch` with a range check (`xherd-daemon::settings`), and to the Settings page. Settings are saved to `config.json` and most apply at the next daemon start; `restartNeeded` tells the UI.

**Add a plugin** — a folder with `xherd-plugin.json`. Built-ins are embedded from `plugins/` through `xherd_plugin::builtins()`; external ones are loaded from the data root and override built-ins by name.

**Change the database** — append a migration in `xherd-store/src/schema.rs`; never edit an existing one. The store copies the database with `VACUUM INTO` before migrating.

**Change the UI** — add the type to `lib/types.ts` (mirrors the wire JSON), call through `lib/api.ts`, and register panels in `lib/panels.ts`. Only screens for features that exist are shown.

## Docker image

```sh
docker build -f deploy/docker/Dockerfile -t ghcr.io/xionjames/xherd:dev .
cd deploy/compose && docker compose up -d --build
```

Inside the container, `XHERD_SANDBOX=1` enables toolchain installs and installs the xherd commands at every start. Tests never set `Options::sandbox`, so they cannot write into a real home.

## Releases

Push a tag `vX.Y.Z` on a commit of main (`git tag v0.2.0 && git push origin v0.2.0`). `.github/workflows/release.yml` refuses a tag not on main, runs CI, then publishes the Linux binaries (static musl, x86_64 and arm64, as `.tar.gz`, `.deb` and `.rpm`, the packages from `deploy/nfpm/nfpm.yaml`; no macOS binaries yet) on a GitHub release and the image `ghcr.io/xionjames/xherd` (amd64 + arm64) as `X.Y.Z`, `X.Y`, `X` (not for 0.x) and `latest`. A tag with a suffix (`v0.2.0-rc.1`) is a prerelease and only gets `X.Y.Z-suffix`. Nothing is published from main.

## Git workflow

One commit per working piece, on a branch per piece of work; tests and clippy pass at each commit. Do not commit `target/`, `ui/dist` or local `.env` files.

## Further reading

- [XAP reference](xap.md): the agent protocol
- [Add xherd support to your SDD](integrate-your-sdd.md)
- `packs/xherd/FORMAT.md`: formats of the documents agents write
- `ROADMAP.md`: what is left, by theme
