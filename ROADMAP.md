# Roadmap

High-level view of the work, done and pending. `[x]` = done, `[ ]` = pending.
The sources of truth stay where they were: the milestone checklist in
[`doc/design.md`](doc/design.md) §18, the roadmap in [`doc/spec.md`](doc/spec.md)
§21 and the status notes in [`AGENTS.md`](AGENTS.md). This file groups the work
by theme; update it together with them.

## Foundations (M0, M1)

- [x] Skeleton: workspace, config and paths, store and migrations, supervisor and actors, scheduler, plugin loader, git/tmux wrappers, `xherd hook/report` with the offline queue
- [x] One agent end to end: xherd v0, worktree + `.xherd/`, Claude Code in tmux, XAP gates, hash approvals, fresh sessions, commit gates, ship, draft PR via GitHub

## M2 UI

- [x] Svelte UI on the panel registry: Herd, Agent page, Settings, Setup checks, Welcome
- [x] Hash approvals from the UI, websockets, terminal, diffs
- [x] Welcome: plugins switched on/off and configured, with a Claude login step; at least one of each kind
- [x] Commits: xherd runs the approved commit (editable message, agent fixes it if git refuses); Commits view with message and files; Changes tabs: uncommitted / all changes / one commit
- [x] Agent page: Terminal first and default, vertical menu, one-row "Your call" band, Approve only on the tab that holds what is approved, "Open the terminal" for blocked agents
- [x] Gate documents: highlight what changed after a change request (spec, design, tasks…), without keeping version history
- [x] Open questions: a text box per question to answer them inline
- [x] Board (tasks of all agents by status) and Review with "Mark seen"; the notification bell is in
- [x] Time (work log: agent and user time per day and ticket, delivery metrics) and TODOs (reminders, once or repeating)
- [x] Board stage cards (by role), swimlanes (repository, agent) and TODO cards; TODO condition checks (runtime plugin `data.terminal.check`, a read-only headless run ending in `XHERD_RESULT`)
- [ ] Board cards declared by packs and profiles (R-1101), checks for runtimes other than Claude Code (they need a `check` command in their plugin)
- [x] Live reload of the config: saving applies everything except port, bind, concurrency, workspace, tmux socket and plugin settings (`SettingsHandle`); editing `config.json` by hand still needs a restart
- [ ] Plugin secrets and plugin install from the UI

## M3 Docker-first

- [x] Image with version managers, `xherd init`, `docker init/doctor/update`, toolchain resolution
- [x] Open-in (URL schemes, Dev Containers, companion)
- [x] Git access setup and doctor
- [x] Commit identity: global, and by repository owner (personal and work on the same host)
- [ ] SSH agent forwarding for Colima, openvscode-server (R-705), `xherd login`
- [x] Block an agent on git auth failures (R-688): an auth error on the screen or from a host command blocks a live agent with reason `git access` (UI: "Fix git access"); the band shows the doctor's first failure and fix (`GET /agents/{id}/git-access`)
- [ ] Profile `toolchain:` / `setup:` (R-654; profiles exist, these fields do not yet) and the Welcome toolchain step (R-653; each toolchain plugin's page in Settings → Plugins has defaults, a version dropdown and installs)

## Agents that survive and recover

- [x] First command typed again until the runtime accepts a prompt
- [x] Container restart: agents are parked at their gate or resumed, not failed
- [x] Resume button (also from `error`): continues the last Claude conversation when its transcript exists, else a new session
- [x] Auto-resume a vanished session, at most twice per hour
- [x] Fewer permission prompts: pack commands readable, plain `xherd` calls with allow rules, `edits` as the default mode
- [x] Plugins switched off apply live to toolchains, the host plugin and the trust writer
- [x] Pause with a handoff (R-026): the agent is asked for a note, the session ends after it or after `pauseTimeoutSec`; resume starts a fresh session from the note
- [x] Kill switch (R-032): `xherd pause --all`, `POST /agents/pause-all`, "Pause all" in the Herd; immediate, no handoff (also for the budget pause, on purpose)
- [ ] Background-task guard (R-024); runtime change between steps (R-025)
- [ ] Recovery beyond a lost session: retry a failed start; optionally a "diagnosing" session told what failed
- [ ] Check the Resume button on `parked` agents (no Resume transition from `parked` in the lifecycle table)
- [ ] xherd could also run other mechanical git/test steps itself (e.g. the task `verify` command, pushing stays the agent's until guardrails)
- [ ] Per-agent permission mode (`ask` / `edits` / `auto` / `all` exist; new agents always use `edits`)

## Context and flow quality

- [x] Each step starts from `memory.md` (state, decisions, code map) instead of re-exploring the code
- [ ] Measure the token savings per step; validate the size and accuracy of the code map
- [ ] Pack JSON schemas for gate documents (R-204); `views`, `extends`, `parse` for external packs
- [ ] More flow packs: PR review, research/design; multi-repo tickets
- [ ] `xherd memory propose` and the memory panel

## Git, accounts and credentials

- [ ] A token per owner for a second account on the same host (credentials are one per host today)
- [ ] "Use my GitHub account" button that fills name and email from `gh`
- [ ] Native installs: include the managed gitconfig from `~/.gitconfig` automatically or guide it

## M4 → v1.0

- [x] Secret tripwire on commit proposals (R-117) and the pre-push guard (R-1204), for agents created from now on
- [x] Code tripwires: `sensitivePaths` and `maxDiffLines` global settings (R-115); scope fence is a refusal (R-116)
- [x] Repo profiles, first slice: matched by remote URL then path; base branch, autonomy, `sensitivePaths`, `maxDiffLines`, per-agent token budget, test command tripwire (R-115) and repo instructions, with the global values as defaults; Settings → Repo profiles, JSON export/import (R-1407)
- [ ] More profile fields: commands, untracked-file copy (R-005a), `toolchain:`/`setup:` (R-654), runtime/model per step, plugin setting overrides, spec folder mode, default flow/pack, ticket prefix, tool permissions; the daily budget and a cost estimate per profile
- [x] Untrusted-text blocks (R-1201): `untrusted::wrap` and the block format in the rules; wrap PR comments and ticket text with it when those arrive (PR follow-up)
- [x] Max session length (R-1206): `maxSessionMinutes` pauses a session that runs too long; asks for a handoff first
- [x] `xherd security check` (R-1210), first version; autonomy and budgets are in; the branch protection finding waits for the host command (`security::PENDING`)
- [x] Token budgets (R-1205), global until profiles; per-profile budgets and a cost estimate come with profiles
- [x] Review→fix loop with `maxReviewLoops` (R-120); the review gate is held while review tasks are open
- [x] R-114 first half: findings in `review.md` are Markdown with a field list; major/blocker findings not fixed hold the review gate; severity colours in the UI
- [x] Daemon/agent user split in the container (R-1203, branch `m4-user-split`): `xherd runner`, `xherd-daemon` user and volume, loopback is not the user; **check on Linux and Docker Desktop** (file permissions are not enforced on Colima's mounts)
- [x] Guardrails: "Viewed" marks per file in the Changes panel; Setup check `runner` pings the runner on every run
- [x] Backups and restore (R-1401, R-1403…R-1405): `xherd backup create|verify|restore`, `/backups` API and Settings → Backups, a backup before `xherd docker update`; the restore can move agents to another computer (`--workspace`, `--map`); secrets and logins are left out
- [ ] Scheduled backups with keep-N and a destination command (R-1402), restore inside the container, backups before database migrations (R-1305), encrypted secrets/logins (`age`)
- [x] Autonomy `ask` / `important` (R-109…R-111) with `maxAutoApprovalsInRow` (R-120), per agent, changeable live; the ship stage's auto-approved variant (`--no-ticket-update`) is only reachable at `never` (v1.x), since ship is critical
- [x] "To review" list of auto-approvals (R-111): `GET /reviews` and the page `#/review`; a mark-as-reviewed state and the agent's stated assumptions come with the v1.x views
- [x] PR polling: state, checks and mergeability of a watching agent's PR every `prPollMinutes`; CI and conflict notifications; merged/closed finishes the agent; shown in the PR panel
- [x] PR comments into the agent (R-304): new comments saved wrapped as untrusted data, agent queued with `/xherd:pr-review`, approval hands on `/xherd:pr-apply`; replies are not posted by xherd yet
- [x] PR decisions and replies (R-306): per-comment fix/reply/skip with a note in the PR panel; replies posted by the host on approval
- [x] `pr-template` (R-153): templates looked up on entering the ship stage, `pr-templates.json` read by the `ship` prompt
- [ ] Stage hooks (R-401…R-404), including `pr.merged` (R-307): v1.x, with profiles / pack `stageHooks`
- [x] Notifications: typed (one per agent and kind), the bell and browser notifications in the UI, ntfy from Settings → Notifications
- [x] Per-kind notification filter (`mutedNotifications`) and daemon-side desktop notifications (`desktopNotifications`: `osascript` / `notify-send`, native installs)
- [ ] Notifier plugins, replacing an earlier ntfy message, Windows desktop notifications

## v1.x

- [ ] Codex and opencode runtimes
- [x] To review and Work log views (per-ticket cycle time, review loops and rework)
- [x] Kanban Board: stage cards, tasks, TODOs, swimlanes
- [ ] Pack-declared Kanban cards (R-1101)
- [ ] Toolchain plugins and the Welcome installer; plugin install from the UI with a catalogue
- [x] TODO reminders
- [ ] Daily budget per profile, `never` autonomy (R-1209: one-time confirmation; the policy table already has it, creation and `autonomy` refuse it)

## Later

- [ ] ACP driver per runtime, selectable per agent
- [ ] Server / Kubernetes deployment; VS Code Remote-SSH entries
- [ ] Electron desktop app (tray, notifications, built-in host companion)
- [ ] Panel plugins; per-agent containers with egress allowlists; parallel task execution inside one agent
- [ ] `xherd mcp`

## Verify before relying on it

- [ ] `claude auth status` exits non-zero when logged out (only run logged in)
- [ ] `claude --resume` finds its transcript after a container recreate (assumes the home volume persists)
- [ ] The new UI layout in a browser at several widths
- [ ] A full agent run (spec → ship) with the plain-`xherd` prompts, the allow rules and the `edits` default
