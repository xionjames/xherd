# AGENTS.md

Guidance for coding agents (Claude Code, Codex, opencode…) working in this repository. **Keep this file current**: when you add a crate, a command, a convention or a gotcha, update it in the same change.

## What this is

xherd: a daemon (Rust) that runs, watches and steers many coding agents, each in its own git worktree and tmux session, with approval gates. Read before changing behaviour:

- `doc/spec.md` — what (requirements `R-xxx`)
- `doc/design.md` — how (crates, architecture, milestones §18, decisions §19)
- `doc/xap.md` — agent protocol (`xherd hook|report|gate|task|commit …`)

Decisions in `doc/design.md` §19 are settled; do not re-open them in code. Cite requirement ids (`R-009`) in doc comments where code implements one.

## Status

**M2** (UI) is on branch `m2-ui`: the web UI (Herd, Agent page with your-call band and panels, terminal, changes, Settings, Setup checks, Welcome, New agent), `/ws` and `/ws/term/:id`, hash approvals from the UI, `/checks`, `/config`. Not in M2: Board, Review, TODOs, Time, bell, phones and Open-in are hidden until their milestones; PR follow-up in the UI (M4); plugin secrets and install from the UI; live config reload (settings are saved to `config.json` and apply at the next daemon start, `restartNeeded` says so).

M0 (skeleton) and **M1** (one agent end to end) are merged; progress is tracked as checkboxes in `doc/design.md` §18. M1 was checked end to end with real Claude Code and `gh` (draft PR opened on a real repository). M1 scope decisions: approvals from the CLI only (the UI comes in M2); xhSpec v0 ships the `feature` flow only. `xherd new` / `POST /api/v1/agents` create agents (worktree + `.xherd/` + Claude Code hooks) that start: the built-in `claude-code` runtime plugin gives the launch argv (`claude`, permission mode `ask`), the daemon starts it in a tmux session named after the agent, and types the flow's start command once the runtime's `session-start` hook arrives. Approving a gate starts a **fresh session** (`freshSessions`, default on; R-023): the agent window's process is respawned with `agent.launch` and the next command is typed on the new `session-start`; with it off, the command is typed into the current session. Resume starts with the pack's `resume` command unless a next prompt is pending (R-027). Commit gates are reviewed with `xherd diff <agent>` / `GET /agents/:id/diff` before `xherd approve`; at the ship gate (`pr.md`) the same diff shows the branch as it will be pushed (`<agent.base>...HEAD`). Approving the ship gate hands on `/xhspec:ship`, and the **agent** pushes and opens the draft PR (the daemon never pushes), then reports `xherd pr opened`; a flow agent that stops idle with `pr_url` set and no gate ends its session and goes to `watching` (R-301). If it stops in the ship stage without reporting one, the daemon asks the host plugin (`pr-for-branch`, built-in `github` through `gh`; R-302). `xherd ls` shows the PR once no gate is open. There is no autonomy in M1, so the ship stage's `autoApproved` variant (`--no-ticket-update`) is never used; the pre-push guard (R-1204) is a guardrail (M4) and PR polling is PR follow-up (M4); code tripwires on commits (R-115…R-117, incl. the secret tripwire) and `diff_viewed` marks are M4/M2. Not done yet: pause with a handoff (R-026, pause just ends the session), the background-task guard of R-024 (no data source), runtime change between steps (R-025, one runtime), resuming the runtime's own session.

## Layout

| Path | Contents |
|---|---|
| `crates/xherd-core` | Domain: `Status` + lifecycle table (`status::next_status`), `Agent` (`fire(Trigger)`), `Report` + `report::apply_report` (sequence rules R-009, no revival R-008), flow types (`Gate`, `Task`, `Commit`, `Role`), `xap` (wire types `XapCall`/`XapCmd`, `Refusal`), `hash::content_sha256` (CRLF-normalised), `new` (`NewAgent` request, pure R-002 checks, `agent.json` and `instructions.md` content), `Paths`, `Config`/`Secrets` (schema migrations, unknown keys kept), `queue` (`hook-queue.jsonl`: reports and report-only XAP calls, sync, file-locked), `exec::run` (the only way to run external commands). Pure and heavily tested (incl. proptest). |
| `crates/xherd-store` | SQLite (WAL, one writer thread, read-only pool), forward-only migrations in `schema.rs` (append only), `VACUUM INTO` copy before migrating. |
| `crates/xherd-daemon` | `Daemon::start` (config, user token, store, plugins, supervisor, queue replay, reconcile, scheduler); `create` (`Daemon::new_agent`: checks incl. the runtime plugin, `git worktree add --no-track`, `.xherd/`, the runtime's hook file, excludes, `agent.launch`, cleanup on failure); `install` (`Daemon::install`: pack commands written per runtime, hashes in the `installs` table, user-edited files kept with a `.xherd-new` beside them); `supervisor` (slots, start queue, port blocks); `actor` (one per agent: `Msg::{Report, User, Xap, Tick, WorkDone}`; `on_xap` observes files/git, then calls `xherd_flow::xap::apply`; `approve` writes `.xherd/ledger/approvals.jsonl` (fsync) then the `approvals`/`reviewed` tables, then hands on the next command (`fresh_session`: `Driver::respawn` + `agent.next_prompt`, or typed when fresh sessions are off or the agent has flow `none`); `Resume` sets `next_prompt` to the pack's `resume` command when none is pending; a stop without a declared gate runs `infer_gate`; after a stop, `after_ship` moves an idle flow agent with a PR URL to `watching` (R-301), or in the ship stage without one spawns the host lookup (`Env.host`, built by `host_plugin` at start; answer back as `WorkResult::PrLookup`, events `pr-found` / `pr-lookup-failed`); a `session-start` report types `agent.next_prompt` and clears it); `driver` (`Driver` trait, `TerminalDriver` on tmux, `FakeDriver` for tests); `scheduler` (all periodic jobs); `Daemon::diff` (what the user reviews: the open commit proposal's files vs HEAD plus the uncommitted files left out of it; at a ship gate the branch `<base>...HEAD` (`branch` field) plus uncommitted files that would not be pushed; else every uncommitted change; read-only git, runs outside the actor); `support` (clock, change hub, tokens); `checks` (`Daemon::checks`: git, tmux ≥ 3.3, every plugin's checks (required only for the default runtime; a manifest `fix` shows as a command), runtime trust of the worktrees folder, xhSpec commands installed; `fix_check`: `pack` installs the commands, `trust` starts the default runtime in the worktrees root in tmux session `xherd-setup-trust` (reused if running) that the UI shows through `/ws/setup/trust`, so the user accepts the runtime's own trust dialog from the browser); `settings` (`ConfigPatch`, `deny_unknown_fields`, range-checked `apply`; `config_view` = saved vs running). |
| `crates/xherd-api` | axum router: `/` and `/assets/*` (the UI, `ui.rs`: `ui/dist` through `rust-embed`, read from disk in debug builds, embedded in release, a "not built" page when missing), `/api/v1/ws` (`ws.rs`: hub `Change`s as JSON, closes a lagging client), `/api/v1/ws/term/{id}?cols=&rows=` and `/api/v1/ws/setup/trust` (`term.rs`: binary frames = terminal bytes both ways, text `{"type":"resize",…}`), `/api/v1/{healthz,state (incl. `flows`: every pack flow's stages),agents (GET, POST),agents/{id},agents/{id}/diff (GET),agents/{id}/docs (GET: spec-folder `*.md`, gate files first),agents/{id}/file (GET `?path=`: text, `doc_sha256`, last approved text),agents/{id}/{pause,resume,cancel,send,approve,request-changes},install,config (GET, PUT),checks (GET),checks/{id}/fix (POST),hooks,xap}`; `User` and `AgentAuth` (agent id + scopes) extractors. `/xap` answers `{ok, result}` or `{ok: false, code, message}` (422). |
| `crates/xherd-plugin` | `xherd-plugin.json` manifests, `Registry` (built-ins overridden by external, broken listed), settings/secrets resolution, command protocol (`run_command`, `run_typed`, `run_hook`, `run_checks`). `builtins()` = manifests embedded from `plugins/`. `host`: `Host` (plugin + resolved `Context`; no `Debug`, it holds secrets) with `pr_for_branch` (built-in `github`: `gh pr list --head <branch> --state all --json url --limit 1` in the worktree, setting `gh` = binary, secret `token` → `GH_TOKEN`; an external `github` plugin answers `{url}` or `null` through the command protocol, input `{branch, worktree}`). `runtime`: a runtime plugin's `data.terminal` (`Terminal`: launch argv, `permissionModes`, `hooks` file + event map) and `merge_hooks` (Claude Code settings JSON, xherd entries tagged `# xherd`). |
| `plugins/claude-code` | Built-in runtime plugin manifest (R-504, R-511). |
| `plugins/github` | Built-in host plugin manifest (settings `gh`, secret `token`; checks `gh --version`, `gh auth status`); its commands run in Rust (`xherd_plugin::host`). Only `pr-for-branch` exists so far; `pr-status`, `pr-checks`, `pr-comments`, `pr-reply`, `pr-template` come with PR follow-up (M4). |
| `crates/xherd-flow` | Flow packs: `xherd-flow.yaml` manifest (`Pack`, `Flow`, `StageDef`, `next` rules with the fixed `Condition` vocabulary and a `Facts` trait), `template::render` (`{name}` placeholders; `${…}` kept), `frontmatter::parse`, `tasks::parse` (xhSpec `tasks.md`), `approve` (next command after a gate, change-request text, `infer_gate` for undeclared or edited documents, `DirFacts`). `xap::apply` is the pure XAP validation (refusal codes, xap.md) over what the actor observed (`Observed`); `xap::doc_path` canonicalises document paths (`spec.md` = spec folder, `./x` = worktree root). `xhspec()` returns the built-in pack embedded from `packs/xhspec/`. `install`: `XHSPEC_FILES` (the pack's files, embedded), `Pack::plan_install` (command files rendered for a runtime with `{{prefix}}`, `{{args}}`, `{{dir}}` = install folder, `{{xap}}` = `XAP_SHELL`, `{{format}}` = `FORMAT.md`; destinations with `${VAR:-default}` and `~/` expanded). |
| `packs/xhspec` | The built-in xhSpec pack v0: `xherd-flow.yaml` (feature flow), `FORMAT.md` (document formats; `tasks.md` as `tasks::parse` reads it), `commands/*.md` (Claude Code command prompts: feature, design, tasks, next, commit, review, ship, resume, handoff, status, rules). No JSON schemas yet (optional, R-204; `gate open` only checks that frontmatter parses). |
| `ui` | Web UI (M2): Svelte 5 + Vite + TypeScript, pnpm. `src/app.css` = design tokens + component classes; `lib/` (`store.svelte.ts`: `/state` + `/ws`, full reload on reconnect; `router.svelte.ts`: hash routes `#/path?query`; `api.ts`; `types.ts` mirrors the wire JSON; `format.ts`, `flow.ts`), `components/` (bar, herd strip, your-call band, stepper, dialogs), `pages/`, `panels/` + `lib/panels.ts` (panel registry: type → component; the built-in agent view lists the tabs, flow `none` keeps only Terminal/Changes/Pull request/Activity). Agent-written Markdown goes through `lib/markdown.ts` (marked + DOMPurify), never raw. Only screens for features that exist are shown. |
| `crates/xherd-term` | `Tmux` (exact `=name` targets, optional `-L` socket, `attach_argv`), `ScreenRules`, `pty::PtyBridge` (`portable-pty`: a `tmux attach-session` client in a PTY for browser terminals; output on a channel, `input`, `resize`; drop kills the client, never the session). |
| `crates/xherd-git` | `Git`: worktrees (`--no-track`), diffs (`diff_proposed`: given files vs HEAD through a temporary index), commit files, bundles, `info/exclude`. |
| `crates/xherd-cli` | Binary `xherd`: `daemon`, `status`, `new <KEY> [--slug] [--flow] [--repo] [--base] [prompt]` (sends an absolute repo path), `ls`, `install [--runtime] [--pack]`, `pause|resume|cancel|send|approve <agent>`, `changes <agent> <comment>`, `diff <agent>` (diff on stdout, what it covers on stderr; the branch at a ship gate; `--json` for the raw answer), `ls` (with the open gate, else the PR URL), `hook <event>`, `report …`, and XAP (`stage enter`, `gate open`, `ask`, `tasks sync`, `task start|done|block`, `commit propose|done|drop`, `pr opened`, `handoff written`, `issue add`; global `--json`). `memory propose` and `xherd mcp` are not implemented yet. |

## Running it

```sh
XHERD_HOME=/tmp/xh cargo run -p xherd-cli -- daemon     # JSON logs on stderr; XHERD_LOG=debug for more
XHERD_HOME=/tmp/xh cargo run -p xherd-cli -- status
XHERD_HOME=/tmp/xh cargo run -p xherd-cli -- install  # /xhspec:* commands into ${CLAUDE_CONFIG_DIR:-~/.claude}/commands/xhspec/
```

One-time step before `xherd new` with Claude Code: trust the worktrees folder (`<workspace>/worktrees`; default workspace `~/xherd`), or `new` refuses with `runtime_untrusted`:

```sh
mkdir -p ~/xherd/worktrees && cd ~/xherd/worktrees && claude   # choose "Yes, I trust this folder", then quit
```

The UI (needs Node and pnpm; cargo does not run it):

```sh
cd ui && pnpm install && pnpm build     # ui/dist, served by the daemon at http://127.0.0.1:4740/
cd ui && pnpm dev                        # Vite dev server, proxies /api to XHERD_URL (default 127.0.0.1:4740)
cd ui && pnpm check                      # svelte-check (types)
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
- External commands only through `xherd_core::exec::run` (argv, no shell, timeout + process-group kill, env allowlist, redaction). One exception: the interactive tmux client of a browser terminal (`xherd_term::pty`), still with an env allowlist.
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
- YAML is parsed with `yaml_serde` (the maintained serde_yaml). It expects `!Tag` syntax for enums; pack enums written as one-key maps (`Condition`) go through a raw struct with `try_from`.
- Pack manifests use `deny_unknown_fields`: fields not implemented yet (`views`, `extends`, `parse`…) are rejected, not ignored.
- XAP calls are de-duplicated by call id through the events table (`xap.<cmd>` events use the call id). A replayed call answers `{"changed": false, "duplicate": true}`.
- XAP observation (file reads, git) runs inline in the actor; keep it to small local reads.
- Gate documents are hashed with `xherd_flow::approve::doc_sha256`, never `content_sha256` directly: in the tasks-stage document the `- status:` / `- evidence:` lines of task sections are dropped first (`tasks::without_progress`), so progress does not re-open the tasks gate (R-107). Use it at every place that records or compares a gate-document hash.
- Approval hashes what is on disk at approval time (the user approves what they read), not the hash recorded at `gate open`. Commit-gate files are recorded as `./<path>`; deleted files as `deleted`.
- On start the daemon re-imports every agent's `approvals.jsonl` into the table (`INSERT OR IGNORE`): the file wins.
- A fresh session respawns only the `agent` window (`respawn-window -k`); the tmux session, its environment (`XHERD_*`, the agent token) and other windows stay. The new process gets the variables because `new-session -e` puts them in the session environment.
- Approving a parked agent stores the next command in `agent.next_prompt` and queues it; it is typed at the next `session-start` hook, like a new agent's first command.
- Worktrees live at `<workspace>/worktrees/<repo dir name>/<agent>`; workspace = `config.workspace`, else `$XHERD_WORKSPACE`, else `~/xherd` (design §8.1). Tests must set `workspace` in `config.json` or they write to the real `~/xherd`.
- `xherd new` / `POST /agents` require the runtime to trust the worktrees root (`data.terminal.trust` in the runtime plugin; Claude Code: `${CLAUDE_CONFIG_DIR:-~}/.claude.json`, `projects[path].hasTrustDialogAccepted`, an ancestor counts). xherd only reads that file, never writes it. Tests that create agents must point `trust.file` of the built-in `claude-code` manifest at a fixture (see `tests/create.rs` setup), never the real `~/.claude.json`.
- `.xherd/` is excluded through the repo's shared `.git/info/exclude` (one line covers every worktree).
- A new agent's first command (the flow's `start`, or the prompt for flow `none`) is stored in `agent.next_prompt` and typed when the runtime's `session-start` hook reports, with the pack's `slashEnters` when it starts with the pack prefix. No hook (e.g. Claude Code still shows its folder-trust dialog: attach with `tmux [-L socket] attach -t =NAME` and accept) = nothing typed yet.
- The runtime's hook file (`.claude/settings.local.json`) is written at creation, not at each launch: xherd's entries (command ends with `# xherd`) are replaced, the user's kept; invalid JSON refuses creation. Hook commands use the daemon's absolute `xherd` path; the token and agent id come from the session environment.
- `Options::new` loads the built-in plugins (`xherd_plugin::builtins()`); clear `builtin_plugins` to test a missing runtime.
- A new file in `packs/xhspec/` must be added to `xherd_flow::install::XHSPEC_FILES` (a test compares the folder with the list).
- Prompts teach XAP as `"$XHERD_BIN" <call>` inside backticks on one line, with concrete example values: the CLI test `prompt_calls_parse` parses every such call with clap, and `every_template_has_a_prompt` checks each manifest command has a `commands/<name>.md`.
- `xherd install` runs in the daemon (`POST /api/v1/install`), so commands land where the agents run (user level, per the pack's `install.<runtime>.commands.to`). Run it once before the first agent; nothing installs automatically yet. Tests call `Daemon::install` with their own `env` closure: never let a test resolve the real `HOME`.
- `Git::diff_proposed` stages the proposed files into a temporary `GIT_INDEX_FILE` (with `--force`, so proposed ignored files show) and never touches the worktree's index; paths are literal (`--literal-pathspecs`). Output is capped at 4 MiB by `exec::run` (silently truncated).
- `approve` takes an optional `expect` (document → hash the UI read, from `GET /file`); a mismatch refuses with `changed_since_read`, so a UI approval covers exactly what was shown. The CLI sends none (hashes what is on disk).
- A runtime `Notification` while the agent is stopped (`waiting-input`, `idle`) is an idle reminder (Claude Code: `notification_type: idle_prompt`, about a minute after a stop) and changes nothing; only mid-turn notifications (permission prompts) or `report --state blocked` set `blocked`. A blocked agent cannot be approved.
- A question gate (`xherd ask`) is not approved: `xherd send` while it is open is the answer, clears the gate and logs `user.answer`.
- `Tmux::send_text` pauses 300 ms before each Enter: typed text and Enter sent back to back are taken as one paste by Claude Code and the Enter is dropped (the delivery retry then rescues it 10 s later).
- Supervisor messages carry `oneshot` replies straight to the caller; the supervisor never awaits an actor.
- Local browsers get full rights by address (loopback peer, local Host, no proxy headers) **and** a local or absent `Origin`: a request carrying another site's `Origin` is refused (no cross-site requests or websockets).
- Debug builds serve `ui/dist` from disk (rebuild the UI, reload the page); release builds embed it at compile time, so build the UI before `cargo build --release`.
- UI panels key effects on `const id = $derived(agent.id)`, never `agent.id` directly: the agent object is replaced on every websocket update, and reading it in an effect would reconnect terminals and refetch on each change.
- `Change` (hub messages, `/ws`) is tagged `type` (kebab-case) with camelCase fields.
- UI theme: the bar button cycles system → light → dark (`lib/theme.svelte.ts`), sets `data-theme` on `<html>` and keeps the choice in `localStorage` (`xherd.theme`, per browser); `app.css` reads `data-theme` before `prefers-color-scheme`. The terminal stays dark in both.
- UI: `--gate` (coral) means "needs a human" and nothing else; agent hues avoid it. Status is never colour alone (pair with a label or glyph).
- The UI blocks every screen but Setup checks and Settings while a required check fails, and sends a first-run user (`config.welcomed` false in the saved config) to `#/welcome`.
- Built-in plugin checks run their argv as written (`gh --version`), ignoring settings such as `pluginSettings.github.gh`; tests that run `Daemon::checks` keep only a runtime plugin with a harmless check (see `tests/checks.rs`) and pass their own `env` (HOME) so the pack check never reads or writes the real `~/.claude`.
- Tests never call the real `gh`: set `pluginSettings.github.gh` in `config.json` to a fake script (see `fake_gh` in `crates/xherd-daemon/tests/xap.rs`). The branch looked up is the agent name.

## Commits

One commit per working piece on the milestone branch; tests and clippy pass at each commit.
