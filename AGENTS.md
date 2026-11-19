# AGENTS.md

Guidance for coding agents (Claude Code, Codex, opencode…) working in this repository. This file is shared memory: it changes only through `xherd memory propose` at review (the user accepts it and xherd commits it), never inside a task commit. Note in your `memory.md` what should change here.

## What this is

xherd: a Rust daemon that runs, watches and steers many coding agents, each in its own git worktree and tmux session, with approval gates. Read before changing behaviour:

- `doc/internal/spec.md` — what (requirements `R-xxx`)
- `doc/internal/design.md` — how (crates, architecture, milestones §18, decisions §19; the decisions are settled, do not re-open them in code)
- `doc/reference/xap.md` — the agent protocol (`xherd hook|report|gate|task|commit …`)
- Final docs by reader (index `doc/README.md`; layout and rules in `doc/dev/development.md` → Writing docs): `doc/user/` (getting-started, guide, docker, security), `doc/reference/` (cli, configuration, http-api, xap), `doc/extend/` (plugins, flow-packs, integrate-your-sdd), `doc/dev/` (from-source, development, architecture). Update them in the same change as the behaviour they describe: a CLI flag → `cli.md`, a config key → `configuration.md`, a route → `http-api.md`, a user-visible feature → `guide.md`. `README.md` is the presentation page.
- `ROADMAP.md` — what is left, by theme (the milestone checklist in `design.md` §18 is the source of truth; update both).

Cite requirement ids (`R-009`) in doc comments where code implements one.

## Vocabulary

- **Job**: one unit of work (worktree + tmux session + branch). Code: `Job`, table `jobs`, `/api/v1/jobs/…`, `XHERD_JOB_ID`, `.xherd/job.json`.
- **Agent**: a saved persona (`AgentDef`: runtime, model, mode, autonomy, instructions, skills, memory). A flow stage can be assigned an agent; `Job.agents` holds snapshots, `Job.current_agent` the one running.
- **Flow / pack / stage**: a flow pack (`packs/xherd`) defines flows (`feature`, `bugfix`), each a list of stages with gates.
- **Action**: a named command a job registers (`xherd action add`) and the user runs in its own tmux session `xherd-<job>-act-<id>`; `service` or `task`. Code: `xherd_core::action` (`ActionDef`, `JobAction`), `Job.actions`/`port_base`, `Profile.actions`, daemon `actions.rs`, `/api/v1/jobs/…/actions`. Not a flow command or a saved agent.
- **Runtime**: a plugin that provides the coding CLI (`claude-code`, `opencode`, `codex`). The OS user that runs sessions is the "agents' user" (`as_agent`, `XHERD_AGENT_*` are about that user, not saved agents).

## Layout

| Path | Contents |
|---|---|
| `crates/xherd-core` | Pure domain, heavily tested (proptest): `Status` and lifecycle, `Job` (`fire(Trigger)`), reports, flow types, `xap` wire types and `Refusal`, `hash::content_sha256`, `new` (creation request and checks), `Paths`, `Config`/`Secrets` (schema migrations, unknown keys kept), `queue` (`hook-queue.jsonl`), `untrusted::wrap` (untrusted text for jobs, per-message token), `exec::run` (the only way to run external commands), `agent_def`, `resolve`, `autonomy`, `profile`, `pr`, `tripwire`, `guard`, `todo`. |
| `crates/xherd-store` | SQLite (WAL, one writer thread, read-only pool), forward-only migrations in `schema.rs` (append only), `VACUUM INTO` copy before migrating. |
| `crates/xherd-daemon` | `Daemon::start`, `create` (new job: worktree, `.xherd/`, hook file, launch argv), `actor` (one per job), `supervisor` (slots, start queue), `driver` (tmux or fake), `scheduler`, `checks`, `git_access`, `open_in`, `repos`, `settings`, `agents`, `skills`, `profiles`, `backup`, `worklog`, `notifier`, `todos`, `tickets`, `plugin_dirs`. |
| `crates/xherd-api` | axum router: the UI (`ui/dist` through rust-embed), `/api/v1/ws`, terminal websockets, REST routes, `/xap`. `User` and `AgentAuth` extractors. |
| `crates/xherd-plugin` | `xherd-plugin.json` manifests, `Registry`, command protocol, `host` (PR/ticket operations), `toolchain`, `runtime` (launch, permission modes, hook files, resume, usage, models), `view` (Herd view plugins), `tracker`, `git`, `gitlab`. Built-ins are embedded from `plugins/`. |
| `crates/xherd-flow` | Flow packs: manifest, `template::render`, `frontmatter`, `tasks::parse`, `questions`, `review`, `pr_review`, `approve` (next command, `infer_gate`, `doc_sha256`), `xap::apply` (pure XAP validation), `install`, `memory`, `progress`. |
| `crates/xherd-term` | `Tmux`, `ScreenRules`, `pty::PtyBridge` (browser terminals). |
| `crates/xherd-git` | `Git`: worktrees (`--no-track`), diffs, commits, bundles, `info/exclude`. |
| `crates/xherd-cli` | Binary `xherd`: `daemon`, `init`, `new`, `ls`, `install`, job actions, `hook`/`report`, XAP calls, `docker`, `open`/`attach`/`shell`, `credential`, `git doctor`, `toolchain exec`, `backup`, `agent`, `flow`, `terminal-handler`. |
| `packs/xherd` | Built-in flow pack: `xherd-flow.yaml`, `FORMAT.md` (document formats), `commands/*.md` (the prompts). |
| `plugins/*` | Built-in plugins: runtimes (`claude-code`, `opencode`, `codex`), toolchains (`sdkman`, `nvm`, `uv`, `go`, `rustup`), hosts (`github`, `gitlab`, `bitbucket`), trackers (`jira`, `github-issues`, `gitlab-issues`), view (`herd-tiles`). |
| `ui` | Svelte 5 + Vite + TypeScript, pnpm. `src/app.css` has tokens and component classes; `lib/` (state, router, api, pure helpers), `components/`, `pages/`, `panels/`. |
| `deploy/docker`, `deploy/compose`, `deploy/nfpm` | Image, compose files, packages. |
| `.github/workflows` | `ci.yml` (fmt, clippy, tests with tmux, UI checks, image build), `release.yml` (tag → binaries, packages, image). |
| `scripts/demo` | `node scripts/demo/server.mjs [port]` serves `ui/dist` with mock data, no daemon needed (build the UI first); used for screenshots in `doc/assets`. |

## Running it

```sh
XHERD_HOME=/tmp/xh cargo run -p xherd-cli -- daemon     # JSON logs on stderr; XHERD_LOG=debug for more
XHERD_HOME=/tmp/xh cargo run -p xherd-cli -- status
XHERD_HOME=/tmp/xh cargo run -p xherd-cli -- install    # pack commands into the runtime's folder
```

Claude Code must be logged in once (`claude`, then `/login`); xherd marks each worktree trusted itself. A daemon run from source needs `target/debug` on its `PATH`, or the prompts' plain `xherd …` calls fail (tmux takes a session's `PATH` from the client that creates it).

Docker (everything inside the container, only the workspace shared):

```sh
cd deploy/compose
printf 'XHERD_WORKSPACE=%s/xherd\nXHERD_UID=%s\nXHERD_GID=%s\n' "$HOME" "$(id -u)" "$(id -g)" > .env
mkdir -p ~/xherd && docker compose up -d --build     # http://127.0.0.1:4740/
```

UI (needs Node and pnpm; cargo does not run it): `cd ui && pnpm install && pnpm build` (served from `ui/dist`), `pnpm dev` (proxies `/api` to `XHERD_URL`), `pnpm check` (types). Debug builds read `ui/dist` from disk; release builds embed it, so build the UI before `cargo build --release`.

Data root: `XHERD_HOME`, else `$XDG_DATA_HOME/xherd`, else `~/.local/share/xherd`; `config.json` there holds the port (4740), `tmuxSocket` and the user token. Worktrees live at `<workspace>/worktrees/<repo>/<job>`; workspace = `config.workspace`, else `$XHERD_WORKSPACE`, else `~/xherd`.

## Build and test

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

If `cargo` is missing, add rustup's bin directory to `PATH` (Homebrew's is keg-only: `/opt/homebrew/opt/rustup/bin`). Integration tests need `git` and `tmux` ≥ 3.3; tmux tests use a private socket (`-L xherd-test-…`). `[profile.dev] debug = "line-tables-only"` and `incremental = false` keep `target/` small; `cargo clean` is always safe.

Test rules: set `workspace` in `config.json` or tests write to the real `~/xherd`; never resolve the real `HOME` (pass your own `env`); never call the real `gh` (set `pluginSettings.github.gh` to a fake script, see `fake_gh` in `crates/xherd-daemon/tests/xap.rs`); point `trust.file` of the built-in `claude-code` manifest at a fixture, never the real `~/.claude.json`; never set `Options::sandbox`; clear `builtin_plugins` to test a missing runtime.

## Conventions

- Edition 2024; `unsafe_code = "forbid"`; clippy warnings are errors in CI.
- External commands only through `xherd_core::exec::run` (argv, no shell, timeout and process-group kill, env allowlist, redaction). Exceptions: the browser terminal's tmux client, processes that replace the CLI (`toolchain exec`, `attach`, `shell`, `init`), long-lived helpers.
- A job's state changes only inside its actor, through pure functions in `xherd-core`; slow work returns as `WorkDone` tagged with the job's `generation`. A few deliberate inline exceptions exist (commit on approval, `verify:`, XAP observation): keep them to small local work.
- JSON on the wire and in files is camelCase; enum values are kebab-case. `Change` hub messages are tagged `type` (kebab-case).
- Files the daemon writes go through `paths::write_atomic`; secrets are mode 0600.
- Minimal code: no abstraction for a single use; flag unrelated problems instead of fixing them silently.
- Clean room: never copy or name proprietary tools, prompts, texts or design tokens from other products.
- Docs and AGENTS.md: keep them current with behaviour; AGENTS.md only through shared memory.
- Commits: one per working piece; tests and clippy pass at each.

## How a job works

- A job is created with a worktree, `.xherd/` (job.json, instructions.md, spec/, ledger/, hooks/), the runtime's hook file and a pre-push guard. The runtime starts in tmux; the first command (the flow's `start`) is stored in `job.next_prompt` and typed when the `session-start` hook arrives. It stays until the runtime accepts a prompt (`user-prompt-submit`), because Claude Code fires `SessionStart` twice and drops text typed after the first.
- Base freshness: `locate_repo` fetches `origin` for a repository given as a path (a failure is a warn log and the resolve `warning`, never a refusal); `new_job` then fast-forwards a plain local base branch to `origin/<branch>` (`Git::fast_forward_branch`), and leaves it as it is when git refuses (ahead, diverged, dirty, checked out in another worktree). Tests for this use local repos as remotes.
- Steps are new sessions with no history (`freshSessions`, default on): `memory.md` (State / Decisions / Code map / Watch out) carries over, so each command reads it first and rewrites it before its gate. Keep that wording when editing the commands. Approval respawns only the `agent` tmux window; the session environment (`XHERD_*`, token) stays.
- Gates: the job writes documents under `.xherd/spec/`, calls `xherd gate open`, stops; the user approves by content (hash). Approval hashes what is on disk at approval time (UI approvals send `expect` and refuse `changed_since_read`). Always hash gate documents with `xherd_flow::approve::doc_sha256`, never `content_sha256` directly (progress lines and review/PR tasks are dropped from the tasks document so they do not re-open its gate).
- Commit gates: the actor runs `Git::commit_paths` itself on approval (`git add --all -- files`, `git commit --only -m … -- files`, hooks run). Failure leaves git's output in `.xherd/commit-error.txt` and `/xherd:commit` fixes it. During a merge `commit_paths` stages the proposed files and commits without `--only`.
- Shared memory: `AGENTS.md` and `CLAUDE.md` at the root are refused in `commit propose` (`memory_file`, except during a merge). The review proposes the whole `AGENTS.md` (`xherd memory propose`, ≤ 64 KiB); the Shared memory tab diffs it against `HEAD`; accepting commits `AGENTS.md` and a `CLAUDE.md` importing it (`@AGENTS.md`). Refused while the job runs or after a PR exists.
- Autonomy (levels and table in `xherd_core::autonomy`): `Actor::try_auto_approve` runs after a stop at a Docs or Commit gate whose class allows it, with no `needs_human`, no open questions and under the run limit. Tripwires (secrets, sensitive paths, diff size, scope fence, test/lint commands, merges) set `needs_human`. The `pr-review` gate and any role `pr` gate are always human-only: do not relax that.
- Pause with handoff, kill switch, budgets, session limit, PR polling and PR comments, Ask/Finish, worktree removal, backups, TODOs, work log, notifications: see `doc/user/guide.md` and `doc/reference/` for behaviour; the code is in `actor.rs` and the daemon modules above.

## Gotchas

- `xherd hook` is handled in `main` before clap, inside `catch_unwind` with a silent panic hook: it must never print or exit non-zero, and must not start tokio. `xherd report` likewise.
- Sequence numbers from the CLI are `max(last + 1, now in µs)` so counter- and time-based reports stay ordered.
- Hooks without a token, or with the daemon unreachable / 5xx, go to `hook-queue.jsonl` (replayed at start and every minute); 4xx are not queued.
- XAP calls are de-duplicated by call id through the events table.
- Pack manifests use `deny_unknown_fields`; a new file in `packs/xherd/` must be added to `xherd_flow::install::BUILTIN_FILES`. Prompts teach XAP as plain `xherd <call>` in backticks with concrete values (never a variable); `prompt_calls_parse` parses every such call and `every_template_has_a_prompt` checks each command has a prompt. New XAP subcommands go in `XAP_ALLOW` (`create.rs`).
- YAML is parsed with `yaml_serde`, which expects `!Tag` for enums; one-key-map enums go through a raw struct with `try_from`.
- `rusqlite` is pinned to the version `rusqlite_migration` uses; bump them together. `ureq` 3 needs `http_status_as_error(false)`.
- In axum handlers read the peer through the `ConnectInfo` extractor so `MockConnectInfo` works in tests.
- `verify:` must be one runnable shell command (`sh -n` checked); `task done` runs it in the daemon (`ActorSettings::verify_timeout`).
- Commands that must be valid in the job: `gate open` and `commit propose` need a valid `.xherd/spec/memory.md` (four `##` sections, ≤ 80 lines).
- `.xherd/` is excluded through the repo's shared `.git/info/exclude`. The daemon writes only progress lines (`status`, `evidence`, `sha`) of job documents.
- `Git::diff_proposed` uses a temporary index (`--force`, literal pathspecs) and never touches the worktree's; a file over 8 MiB is not read (`TOO_LARGE`) and turns into a tripwire hold.
- A runtime `Notification` at a stop is an idle reminder and changes nothing; a blocked job cannot be approved. A question gate (`xherd ask`) is answered with `xherd send`, not approved.
- Job actions: the actor handles `action.*` XAP before the flow checks; they run only for the user (never agent, never autonomy), after approval of the exact `run`+`cwd` hash, with the job's env minus `XHERD_JOB_TOKEN`. `remain-on-exit` must be set before the command starts (`start_held`: placeholder, then respawn-pane). A running action keeps the job's port block in the supervisor. Static `/jobs/{id}/actions…` routes go before the `/jobs/{id}/{action}` catch-all.
- `Tmux::send_text` pauses 300 ms before each Enter, or Claude Code takes text and Enter as one paste. tmux runs with `-u` and the image sets `LANG=C.UTF-8`.
- Local rights are given by address (loopback peer, local Host, no proxy headers) and a local or absent `Origin`; in the container only `local_peers` (the gateway) counts and the CLI needs its token.
- Two users in the container: `xherd-daemon` and the agents' user; the daemon sends `as_agent` commands and file operations to `xherd runner`. Worktree files the daemon reads must be group/world-readable; use `runner::agent_env` for the jobs' `HOME`/`PATH`. External plugins are copied to `$XHERD_SHARED/plugins` at start.
- `config.json` is saved under `Daemon.config_write`, `secrets.json` under `secrets_write`: hold the lock from load to save. The plugin name `git` is reserved in `secrets.json`.
- Live settings: `Env.settings` is a `SettingsHandle` (take one snapshot per use); a setting that must stay live needs its field in `ActorSettings::from_config` and `Config::apply_live_from`. Only `port`, `bind`, `maxConcurrentJobs`, `workspace`, `tmuxSocket`, `pluginSettings` need a restart.
- Toolchains install only with `XHERD_SANDBOX=1`; the install command waits for Enter on failure so the terminal shows the error.
- The image has no agent CLIs and no Node: with `Options::runtime_install` (the CLI sets it from `XHERD_SANDBOX`) the daemon installs each enabled runtime at start and on switch-on (`runtime_install.rs`), from the manifest's `requires[]` (`version` pin = minimum, `sha256` per arch, `install` argv gets `XHERD_INSTALL_VERSION/ARCH/SHA256`; no literal version in the script). `new_job` refuses `runtime_missing` meanwhile. Tests must not set `Options::sandbox`; use `runtime_install` with a fake requirement (`crates/xherd-daemon/tests/runtime_install.rs`).
- The image has no C compiler on purpose. `plugins/rustup`'s optional `cc` requirement installs a pinned Zig as a `cc` shim that drops `--target=<triple>` and rustc's aarch64-only linker flag (zig rejects the Rust triple). Checked on aarch64 only.
- Version display rules are in `crates/xherd-core/version.rs` (plain std, `include!`d by `build.rs`: no inner `//!` doc comments); `xherd_core::VERSION` is the Cargo version (compat checks), `BUILD_VERSION` what people see. The image has no `.git`: pass `XHERD_VERSION`.
- Edits to `terminal-handler.sh` or `terminal-install.sh` need a `HANDLER_VERSION` bump and the two sha256 in `handler_version_is_pinned_to_the_scripts`.
- `packs/xherd` prompts and the Herd view protocol (`lib/viewPlugin.ts` and `view-kit/xherd-view.js`) must be kept in step with their Rust or UI counterparts.

## Runtimes and plugins

- A job's runtime is chosen at creation (request, profile, then config `defaultRuntime`) and kept in `Job.runtime`. A runtime manifest's `data.terminal` declares launch argv, permission modes (`ask|edits|auto|all`, each with explicit flags; `all` only in the sandbox), hook file format and event map, `trust`, `resume`, `usage`, `modelArgs`, `models`, `check`, `skills`, `promptArgs`.
- Models: precedence `NewJob.model`, profile `models[<runtime>]`, config `runtimeModels[<runtime>]`; names are checked by `xherd_core::model::clean`.
- Check runs (TODO conditions) are headless and must keep their `check` argv read-only.
- opencode: its TUI is not ready when plugins load, so deliver the first prompt with `promptArgs`, never by typing at `session-start`; `--prompt` is ignored with `--session` (use `resume.promptEnv`); `opencode export` must go to a file; subagent sessions must not report stops; the TUI remembers its last model.
- Codex: launched with `--dangerously-bypass-hook-trust`; commands are skills (`$xherd-<name>`, two Enters); `edits` and `auto` modes need network for `xherd …` calls to reach the daemon; Codex's `input_tokens` include cache reads.
- Untrusted text (PR comments, tickets, anything fetched) must be wrapped with `untrusted::wrap` before reaching a job, and sanitised in the UI (`render(…, external = true)`). Notifications never carry comment text or document contents.
- Tests never use the real `gh`/`glab`; a host plugin's own token is kept only for hosts the user named.

## UI

- Job-written Markdown goes through `lib/markdown.ts` (marked + DOMPurify), never raw; mermaid diagrams are drawn by `lib/mermaid.ts` with strict settings. HTML previews use an iframe with an empty `sandbox`: never loosen it.
- Panels key effects on `const id = $derived(job.id)`, never `job.id` (the job object is replaced on every websocket update).
- Palette "Cobalt & Ember": `--accent` is actions and selection, `--gate` means "needs a human" and nothing else. Status is never colour alone. Text/background pairs pass WCAG AA in both themes: recheck when changing a value. No colours or fonts outside `app.css` tokens.
- One component of each kind (`PageHead`, `Panel`, `SaveBar`, `Tabs`, `Seg`, `Callout`, `Dialog`, `FilePath`); one `primary` button per view; never `alert`/`confirm`. Data pages use the full width, form pages are capped (`.stack.mid`, `.stack.narrow`). Under 760px the layout folds (bottom sheets, 40px touch buttons, 16px inputs).
- The UI blocks every screen but Setup checks and Settings while a required check fails, and sends a first-run user to `#/welcome`.
- Routes: Inbox `#/inbox`, Jobs `#/` (jobs `#/jobs/<name>`), Board, Team (Agents, Flows, Skills), TODOs, Time, Settings. `types.ts` mirrors the wire JSON.
- The terminal (`TermView.svelte`, xterm.js) needs Shift+drag to select (Option on a Mac); theme is `data-theme` on `<html>` with `localStorage` `xherd.theme`.
