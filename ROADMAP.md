# Roadmap

High-level view of the work, done and pending. `[x]` = done, `[ ]` = pending.
The sources of truth stay where they were: the milestone checklist in
[`doc/internal/design.md`](doc/internal/design.md) §18, the roadmap in [`doc/internal/spec.md`](doc/internal/spec.md)
§21 and the status notes in [`AGENTS.md`](AGENTS.md). This file groups the work
by theme; update it together with them.

## Foundations (M0, M1)

- [x] Skeleton: workspace, config and paths, store and migrations, supervisor and actors, scheduler, plugin loader, git/tmux wrappers, `xherd hook/report` with the offline queue
- [x] One job end to end: xherd v0, worktree + `.xherd/`, Claude Code in tmux, XAP gates, hash approvals, fresh sessions, commit gates, ship, draft PR via GitHub

## M2 UI

- [x] Svelte UI on the panel registry: Herd, Job page, Settings, Setup checks, Welcome
- [x] Hash approvals from the UI, websockets, terminal, diffs
- [x] Welcome: plugins switched on/off and configured, with a Claude login step; at least one of each kind
- [x] Commits: xherd runs the approved commit (editable message, job fixes it if git refuses); Commits view with message and files; Changes tabs: uncommitted / all changes / one commit
- [x] Job page: Terminal first and default, vertical menu, one-row "Your call" band, Approve only on the tab that holds what is approved, "Open the terminal" for blocked jobs
- [x] Gate documents: highlight what changed after a change request (spec, design, tasks…), without keeping version history
- [x] Open questions: a text box per question to answer them inline
- [x] Open questions read from the gate documents' `## Open questions` section and answered as inline comments on their list items, sent with the other comments (R-105a)
- [x] Board (tasks of all jobs by status) and Review with "Mark seen"; the notification bell is in
- [x] Time (work log: job and user time per day and ticket, delivery metrics) and TODOs (reminders, once or repeating)
- [x] Board of work units (stages, then tasks, by Pending / In progress / Done), swimlanes (job, repository, workflow) and TODO cards; the Herd's By stage layout (jobs in their flow's stage columns, a lane per workflow); TODO condition checks (runtime plugin `data.terminal.check`, a read-only headless run ending in `XHERD_RESULT`)
- [ ] Board cards declared by packs and profiles (R-1101)
- [x] TODO condition checks for runtimes other than Claude Code: opencode and Codex declare `data.terminal.check`
- [x] Live reload of the config: saving applies everything except port, bind, concurrency, workspace, tmux socket and plugin settings (`SettingsHandle`); editing `config.json` by hand still needs a restart
- [x] Plugin secrets from the UI (write-only, live)
- [x] Plugin install from a git URL: `xherd plugin ls|install|update|remove` and Settings → Plugins (preview, digest, manifest diff on update, recorded `url@sha`), restart by re-exec (`xherd daemon restart`, `POST /admin/restart`), `XHERD_PLUGIN_INSTALL=off`; no live reload, catalogue, archives or `plugin new|test|link`
- [x] Herd views as plugins (#108): plugin kind `view`, a sandboxed frame with protocol v1 and the view kit; built-in `herd-tiles` as the example
- [x] Job names (#114, #115): a lowercase name, an optional ticket (issue URL or key) that prefixes the branch; New job form with More options
- [x] Needs attention and the job page (#120, #122, #124, #127, #128): the ✋ count is only what waits for you, *Approved by xherd* folded last; a gate autonomy will approve is not shown as waiting; one-row header; a file list in Changes
- [x] Gate notices (#133, #135): one notice per gate with what it waits on, none for automatic approvals; the bell says why browser notifications cannot be on, and can send a test
- [x] Checked in real browsers: the bell's texts and re-alerts, the header and file-list layouts (light, dark, phone)
- [ ] Check the Resolve conflicts button in a browser

## M3 Docker-first

- [x] Image with version managers, `xherd init`, `docker init/doctor/update`, toolchain resolution
- [x] Open-in (URL schemes, Dev Containers)
- [x] Settings → Open in (R-706, R-707): hide entries, your own links, Open in terminal (your terminal app in the worktree) through the `xherd-term://` link handler installed once per computer, no companion (#129)
- [x] Terminal handler checked on a real Mac
- [ ] Check the terminal handler on Linux desktops in Chrome and Firefox
- [x] Git access setup and doctor
- [x] Commit identity: global, and by repository owner (personal and work on the same host)
- [ ] Terminal file sharing in Docker: send a file into a job's worktree or fetch one out from the browser terminal, without a bind mount or `docker cp`
- [ ] Port forwarding in Docker: reach a job's dev server (a port it opens inside the container) from the host browser, per job, without publishing ports in the compose file
- [ ] SSH job forwarding for Colima, openvscode-server (R-705), `xherd login`
- [x] Issue trackers: `data.tracker.links` and built-in `jira`, `github-issues`, `gitlab-issues`; the Ticket tab reads the ticket and its comments through `gh` / `glab` (read-only). Jira reading through `acli` is built and the output of `view` and `comment list` is checked against the real CLI. Posting a user-accepted summary comment is built (fake CLIs only). Not done: automatic or stage-hook posting, status transitions, GitHub Projects, giving ticket text to jobs
- [ ] Git providers (proposed, D24, R-689…R-693; steps 1, 2 and 3 done, plus `requires` (R-691: install in the container, instructions natively), token reuse, plugin secrets from the UI and installed versions on the language cards (3: built-in `gitlab` merge requests through `glab api`, the host plugin chosen per job by its remote; **not run against a real GitLab or `glab`**, only a fake `glab`): `data.git` in the `github` manifest and built-in `gitlab` and `bitbucket` with git access only, read by `git_access.rs`; hosts carry a `provider` field): `host` plugins carry the API, credential helper, login, doctor API checks and their own CLI install; hosts bind to a provider; a built-in `gitlab` on `glab`; Gitea and Bitbucket as external plugins. Order: move `gh`/`glab` out of `git_access.rs` into the manifest, provider field in the UI, `gitlab`, the rest
- [x] Block a job on git auth failures (R-688): an auth error on the screen or from a host command blocks a live job with reason `git access` (UI: "Fix git access"); the band shows the doctor's first failure and fix (`GET /jobs/{id}/git-access`)
- [ ] Profile `toolchain:` / `setup:` (R-654; profiles exist, these fields do not yet) and the Welcome toolchain step (R-653; each toolchain plugin's page in Settings → Plugins has defaults, a version dropdown and installs)

## Jobs that survive and recover

- [x] First command typed again until the runtime accepts a prompt
- [x] Container restart: jobs are parked at their gate or resumed, not failed
- [x] Resume button (also from `error`): continues the last Claude conversation when its transcript exists, else a new session
- [x] Auto-resume a vanished session, at most twice per hour
- [x] Fewer permission prompts: pack commands readable, plain `xherd` calls with allow rules, `edits` as the default mode
- [x] Plugins switched off apply live to toolchains, the host plugin and the trust writer
- [x] Pause with a handoff (R-026): the job is asked for a note, the session ends after it or after `pauseTimeoutSec`; resume starts a fresh session from the note
- [x] Kill switch (R-032): `xherd pause --all`, `POST /jobs/pause-all`, "Pause all" in the Herd; immediate, no handoff (also for the budget pause, on purpose)
- [ ] Background-task guard (R-024): done for auto-approval and the move to watching; a user approval still starts a fresh session. Runtime change between steps (R-025)
- [ ] Recovery beyond a lost session: retry a failed start; optionally a "diagnosing" session told what failed
- [x] Resume from `parked` (lifecycle transition `Resume` from `Parked`; test `a_parked_agent_can_be_resumed_and_keeps_its_gate`)
- [ ] xherd could also run other mechanical git/test steps itself (e.g. the task `verify` command, pushing stays the job's until guardrails)
- [x] Ask (#121): `xherd talk` / the Ask box gives a stopped job a session with your text, held open (no auto-approval, no move to watching) until Finish or flow work
- [x] Remove a finished job's worktree (#123): `xherd remove-worktree`, `cancel --remove-worktree`; the branch and record stay
- [x] Per-job tool-permission mode (R-002a): New job form / `xherd new --mode`, repo profile, Settings default; changed live on the job page or `xherd mode` (applies from the next session start); `all` only in the container

## Context and flow quality

- [x] Each step starts from `memory.md` (state, decisions, code map) instead of re-exploring the code
- [ ] Measure the token savings per step; validate the size and accuracy of the code map
- [ ] Pack JSON schemas for gate documents (R-204); `views`, `extends`, `parse` for external packs
- [ ] Loading external flow packs (only the built-in pack loads today) and dynamic flows, so an SDD that makes XAP calls gets gates without a pack of its own (`doc/extend/integrate-your-sdd.md`)
- [x] `bugfix` flow in the built-in pack: analysis, a failing reproduction test committed first, then the fix (#142); the Herd's By stage lanes per workflow and the Board's lanes by workflow
- [ ] More flow packs: PR review, research/design; multi-repo tickets
- [x] `xherd memory propose` and the Shared memory tab (AGENTS.md, #65)
- [x] Saved agents (#141): versioned setups jobs start from, with a skills library, agent memory and its proposals, suggestions after a finished or repeated job, and backups; "agent" now means the saved setup, a run is a job (design §18, D25)
- [x] Agents on workflow stages (#141, D26): agents are personas; a workflow's stages are assigned to agents (Flows page, `xherd flow`), the job snapshots them and switches at each stage start; fallback runtime; Herd By agent
- [ ] Agents on stages, still open: check in a browser and with real runtimes; mixing runtimes in one job (needs R-025); a switch between approve and `stage enter` after a pause
- [ ] Saved agents, still open: check the pages in a browser and a job from an agent with real runtimes; a Skills tab on the agent page; scheduled agent runs

## Git, accounts and credentials

- [ ] A token per owner for a second account on the same host (credentials are one per host today)
- [ ] "Use my GitHub account" button that fills name and email from `gh`
- [ ] Native installs: include the managed gitconfig from `~/.gitconfig` automatically or guide it

## M4 → v1.0

- [x] Secret tripwire on commit proposals (R-117) and the pre-push guard (R-1204), for jobs created from now on
- [x] Code tripwires: `sensitivePaths` and `maxDiffLines` global settings (R-115); scope fence holds the commit (R-116)
- [x] Repo profiles, first slice: matched by remote URL then path; base branch, autonomy, `sensitivePaths`, `maxDiffLines`, per-job token budget, test command tripwire (R-115) and repo instructions, with the global values as defaults; Settings → Repo profiles, JSON export/import (R-1407)
- [ ] More profile fields: commands, untracked-file copy (R-005a), runtime/model per step, plugin setting overrides, spec folder mode, default flow/pack, ticket prefix, tool permissions; the daily budget and a cost estimate per profile
- [x] Untrusted-text blocks (R-1201): `untrusted::wrap` and the block format in the rules; wrap PR comments and ticket text with it when those arrive (PR follow-up)
- [x] Max session length (R-1206): `maxSessionMinutes` pauses a session that runs too long; asks for a handoff first
- [x] `xherd security check` (R-1210), first version; autonomy and budgets are in; the branch protection finding waits for the host command (`security::PENDING`)
- [x] Token budgets (R-1205), global until profiles; per-profile budgets and a cost estimate come with profiles
- [x] Review→fix loop with `maxReviewLoops` (R-120); the review gate is held while review tasks are open
- [x] R-114 first half: findings in `review.md` are Markdown with a field list; major/blocker findings not fixed hold the review gate; severity colours in the UI
- [x] Daemon/agent user split in the container (R-1203, branch `m4-user-split`): `xherd runner`, `xherd-daemon` user and volume, loopback is not the user; **check on Linux and Docker Desktop** (file permissions are not enforced on Colima's mounts)
- [x] Guardrails: "Viewed" marks per file in the Changes panel; Setup check `runner` pings the runner on every run
- [x] Backups and restore (R-1401, R-1403…R-1405): `xherd backup create|verify|restore`, `/backups` API and Settings → Backups, a backup before `xherd docker update`; the restore can move jobs to another computer (`--workspace`, `--map`); secrets and logins are left out
- [ ] Scheduled backups with keep-N and a destination command (R-1402), restore inside the container, backups before database migrations (R-1305), encrypted secrets/logins (`age`)
- [x] Autonomy `ask` / `important` (R-109…R-111) with `maxAutoApprovalsInRow` (R-120), per job, changeable live; ship is critical, so it is never auto-approved
- [x] "To review" list of auto-approvals (R-111): `GET /reviews` and the page `#/review`; a mark-as-reviewed state and the job's stated assumptions come with the v1.x views
- [x] PR polling: state, checks and mergeability of a watching job's PR every `prPollMinutes`; CI and conflict notifications; merged/closed finishes the job; shown in the PR panel with each check (link to the run), the reviewers and their review state, and a local merge check when the host does not say (#75)
- [x] PR comments into the job (R-304): new comments saved wrapped as untrusted data, job queued with `/xherd:pr-review`, approval hands on `/xherd:pr-apply`; replies are not posted by xherd yet
- [x] PR decisions and replies (R-306): per-comment fix/reply/skip with a note in the PR panel; replies posted by the host on approval
- [x] PR *Check now* (#131): `POST /jobs/{id}/pr-check` and a button in the Pull request tab
- [x] Resolve conflicts (#139): `xherd resolve-conflicts`, the pack's `conflicts` command merges the base (never a rebase), the merge commit is always held for a human
- [ ] Run a whole Resolve conflicts round against a real PR
- [x] `pr-template` (R-153): templates looked up on entering the ship stage, `pr-templates.json` read by the `ship` prompt
- [ ] Stage hooks (R-401…R-404), including `pr.merged` (R-307): v1.x, with profiles / pack `stageHooks`
- [x] Notifications: typed (one per job and kind), the bell and browser notifications in the UI, ntfy from Settings → Notifications
- [x] Per-kind notification filter (`mutedNotifications`) and daemon-side desktop notifications (`desktopNotifications`: `osascript` / `notify-send`, native installs)
- [ ] Notifier plugins, replacing an earlier ntfy message, Windows desktop notifications
- [x] GitHub Actions (#41): CI on main and PRs; a `v*` tag on main releases Linux binaries (static musl, `.tar.gz`/`.deb`/`.rpm`) and the multi-arch image `ghcr.io/xionjames/xherd`
- [x] GPL-3.0-or-later licence (#134); builds without git or `XHERD_VERSION` show `dev` (#137)
- [ ] macOS binaries (x86_64 + arm64, `macos-15` runner) and a Homebrew tap (`xionjames/homebrew-tap`, formula pushed by the release with a `HOMEBREW_TAP_TOKEN` secret, depends on git and tmux): built in `release.yml` and `deploy/homebrew/formula.sh`, **not run yet** (first tag, e.g. an `-rc` one, will show; the tap push is skipped for prereleases); a signed, notarized `.pkg` only with an Apple Developer account
- [ ] Rest of R-1313: signatures (cosign), SBOMs, install script; Docker smoke and migration tests in CI

## v1.x

- [x] opencode runtime (branch `runtime-opencode`): choice per job, profile and setting
- [x] Codex runtime (branch `runtime-codex`): skills as commands, hooks, sandbox modes; not yet run end to end with a model
- [x] A model per job (#112): `modelArgs` in runtime manifests, defaults per runtime in Settings (`runtimeModels`) and in repo profiles (`models`), changed per job for its next session
- [ ] A model per step (R-522: per role, plus tier and effort)
- [x] To review and Work log views (per-ticket cycle time, review loops and rework)
- [x] Kanban Board: stages and tasks as work units, TODOs, swimlanes; jobs by stage on the Herd
- [x] Toolchain plugins (`sdkman`, `nvm`, `uv`, `go`, `rustup`) with installs in Settings → Plugins
- [ ] A plugin catalogue (install from a git URL is done)
- [x] TODO reminders
- [ ] Daily budget per profile, `never` autonomy (R-1209: one-time confirmation; the policy table already has it, creation and `autonomy` refuse it)

## Later

- [ ] Dynamic job actions: buttons on the job page and `xherd` calls declared by plugins, flow packs and repo profiles (for example run the app, open a preview, run a script), shown by the job's state and stage
- [ ] Mobile access (R-672, T7): bind xherd beyond loopback with HTTPS and device pairing (`xherd pair`, one-time codes, phones with steering rights only, token rotation revokes them), a layout checked on a phone, push notifications beyond the browser's
- [ ] Windows: native binary and WSL2 guidance
- [ ] ACP driver per runtime, selectable per job
- [ ] Server / Kubernetes deployment; VS Code Remote-SSH entries
- [ ] Electron desktop app (tray, notifications, built-in host companion)
- [ ] Panel plugins; per-job containers with egress allowlists; parallel task execution inside one job
- [ ] `xherd mcp`

## Verify before relying on it

- [ ] `claude auth status` exits non-zero when logged out (only run logged in)
- [ ] `claude --resume` finds its transcript after a container recreate (assumes the home volume persists)
