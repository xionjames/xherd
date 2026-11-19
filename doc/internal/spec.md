# xherd — Product Specification

> Status: **draft v1.0** · 2026-11-06
> Companions: [`design.md`](design.md) (architecture and implementation), [`xap.md`](../reference/xap.md) (agent protocol reference).
> Requirement IDs (`R-xxx`) are stable and referenced from the other documents. Claims not yet verified against the tools they depend on are marked **(verify)**.

---

## Contents

1. Summary
2. Goals, non-goals and MVP scope
3. Vocabulary
4. Scenarios
5. Jobs
6. Gates, approvals and autonomy
7. xherd: the built-in SDD
8. Pluggable SDD: flow packs, step roles, dynamic flows
9. Pull-request follow-up
10. Plugins
11. Declarative views
12. Deployment
13. Guardrails and security
14. Updates and migrations
15. Backups and restore
16. Supporting features
17. User interface
18. CLI
19. Non-functional requirements
20. Beyond coding (vision)
21. Roadmap
22. Decisions

---

## 1. Summary

**xherd** runs, watches and steers a herd of coding agents working on tickets in parallel.

You give xherd a ticket and a repository. It creates an isolated git worktree and a terminal session, starts a coding-agent CLI (Claude Code, Codex, opencode…) with a **spec-driven development (SDD) flow**, stops the job at **approval gates**, lets you approve or ask for changes from a web UI (browser, phone, later a desktop app), and follows the resulting pull request until it is merged.

Key properties:

- **Many jobs in parallel**, each in its own worktree and branch, with a concurrency cap and a queue.
- **Jobs outlive the daemon**: they run in tmux; the daemon can restart without stopping them.
- **The SDD is pluggable**: xherd ships built in; other SDDs plug in as flow packs or by calling the xherd agent protocol (XAP) from their own commands.
- **You stay in control** through gates, autonomy levels with tripwires, and an audit trail of what was approved.
- **The human reads what matters**: the spec, the diff and the review findings are reviewed by the user, not summarised away — the honest answer to the review-fatigue and skill-erosion complaints around AI coding.
- **Docker-first**, single Rust binary, GPL-3.0-or-later licensed.

## 2. Goals, non-goals and MVP scope

### 2.1 Goals

- G1. Run several coding agents on separate tickets at once, each isolated in its own worktree and branch.
- G2. Keep a human in the loop at the points that matter (spec, design, commits, PR), with adjustable autonomy.
- G3. Make the SDD replaceable: xherd built in, other methodologies through packs or the XAP contract.
- G4. One binary, no runtime dependencies apart from `git`, `tmux` and the agent CLIs; the status hook starts in milliseconds.
- G5. Docker is the easiest way to run xherd, without losing "open this worktree in my IDE".
- G6. Runtime-agnostic: Claude Code, Codex CLI, opencode and others through runtime plugins; live terminal or ACP driver per configuration.
- G7. Coding is the first-class use case, but the core stays domain-neutral where it costs little (workspaces, artifacts, delivery and views are pluggable), so other kinds of work can be added later (§20).

### 2.2 Non-goals

- N1. xherd does not call tracker APIs from its core. Jobs read tickets through MCP or CLIs; tracker plugins may be called only from stage hooks, opt-in.
- N2. No multi-tenant service. **One daemon serves one person**; teams share work through PRs and tickets.
- N3. The daemon makes no LLM calls itself. All model work happens inside runtimes.
- N4. No native Windows build (WSL2 or Docker only).
- N5. No per-job containers in v1: all jobs share one container with all tools.
- N6. **No visual workflow builder.** Flows and views are declared in text (YAML) inside packs and plugins.

### 2.3 MVP scope (v1.0)

The full spec is larger than one release. v1.0 ships a usable tool; the rest follows in minor releases.

| Area | v1.0 | Later |
|---|---|---|
| Runtimes | Claude Code (terminal driver) | Codex, opencode, ACP driver |
| SDD | xherd built in; dynamic flows (§8.6) | External packs with `parse`, panel plugins |
| Hosts / trackers | GitHub; tracker `none` and Jira (link only) | GitLab, Linear… |
| Deployment | Local Docker, native binary (Linux, macOS) | Server/Kubernetes, Electron app |
| UI | Overview, Job page, Settings, Setup checks, Welcome | Kanban, Inbox page, Work log, command palette |
| Autonomy | `ask` and `important` with tripwires | `never` (unattended) after the guardrails have run in practice |
| Plugins | Built-in plugins, `xherd plugin new/test/link` | Install from the UI, catalogue |
| Toolchains | Version managers in the image; per-worktree resolution | Welcome installer, toolchain plugins |
| Other | Guardrails §13, backups §15, updates §14 | TODOs, budgets, notifications beyond browser + ntfy |

## 3. Vocabulary

| Term | Meaning |
|---|---|
| **Daemon** | The long-running xherd service: HTTP API, websockets, web UI, timers. Default port **4740**. |
| **Job** | One coding-agent run for one ticket: workspace (worktree + branch) + terminal session + runtime session(s). Named by a lowercase slug; its branch is `<KEY>-<name>` when it has a ticket key, else the name. |
| **Runtime** | A coding-agent CLI described by a runtime plugin (`claude-code`, `codex`, `opencode`). |
| **Driver** | How xherd talks to a runtime: `terminal` (CLI in tmux, live terminal; default) or `acp` (Agent Client Protocol). |
| **ACP** | Agent Client Protocol: open JSON-RPC-over-stdio protocol between a client and a coding agent (prompts, streamed messages, tool calls, plans, permission requests). |
| **Tracker / Host** | Plugins for the ticket system (link format, key pattern, prompt fragment) and the git host (PR commands, templates). |
| **SDD** | Spec-driven development: a method that moves work through written documents and approvals before and while coding. |
| **xherd** | xherd's built-in SDD. Commands `/xherd:*`. |
| **Flow pack** | A versioned bundle that defines an SDD for xherd: manifest, prompts per runtime, optional schemas, parser and views. |
| **Flow** | A path through a pack's stages (`feature`, `bugfix`), or `none` (no SDD). |
| **Stage / step role** | A pack's step (its own id) and the canonical role it maps to: `explore`, `spec`, `design`, `tasks`, `implement`, `review`, `ship`, `pr`, `custom`. |
| **Gate (✋)** | A point where the job stops and waits for approval: documents, a proposed commit, PR triage, PR replies, or a question. |
| **Autonomy** | `ask` / `important` / `never`: which gates xherd may approve by itself. |
| **Tripwire** | A condition that makes a gate wait for a human despite autonomy. |
| **Fresh session** | After an approved gate, the runtime session is replaced by a new one started with the next command (saves context and tokens). |
| **Parking** | Ending the session of a job that waits at a gate too long, to free a slot. |
| **Watching** | The job's session has ended; xherd polls its open PR. |
| **XAP** | xherd agent protocol: `xherd` subcommands a job or SDD calls to declare gates, tasks, commits, PRs and status ([`xap.md`](../reference/xap.md)). |
| **Workspace** | Where a job works: a git worktree (default) or a plain folder. In Docker mode the workspace root is mounted at the **same absolute path** on the host and in the container. |
| **Private folder** | `.xherd/` in the worktree, excluded from git: job metadata, instructions, SDD documents (by default), ledgers. |
| **Ledger** | Append-only JSONL files: approvals (with document hashes) and discovered issues. |
| **Repo profile** | Per-repository settings matched by remote URL, then by path. |
| **Host companion** | The `xherd` binary on the host when the daemon runs in Docker: attach terminals, open IDEs. |
| **Tier** | Abstract model level (`high`, `medium`, `low`) mapped by each runtime to a concrete model. |

## 4. Scenarios

- **S1. Laptop.** `docker compose up -d`, open `http://127.0.0.1:4740`, start three tickets, approve specs from the phone, open a worktree in IntelliJ with one click.
- **S2. Home server.** Daemon on a server behind HTTPS, browser paired as a device, VS Code in the browser or Remote-SSH for edits.
- **S3. Team SDD.** A team adds the XAP calls to its own SDD commands; xherd shows its gates and tasks without a pack (§8.6).
- **S4. Mixed SDDs.** Ticket SC-1 runs with xherd, SC-2 with another SDD, in the same repo; neither leaves files in the repo; the "why" is in each PR description.
- **S5. Mostly unattended.** Autonomy `important`, a stage hook runs e2e tests before ship, a draft PR appears; the user reviews the auto-approved steps afterwards.

## 5. Jobs

### 5.1 Creating a job

| ID | Requirement |
|---|---|
| R-001 | Input: job name (a lowercase slug, the main identifier), optional ticket (an issue URL or a key; the tracker whose URL pattern reads a URL gives its key), repository (path or URL), optional base branch, flow, prompt, autonomy, tool permissions, runtime/model per step, driver, workspace kind. |
| R-002 | Everything is validated **before** anything is created: name `^[a-z0-9][a-z0-9-]{0,40}$`; ticket: an https URL (one line, ≤ 500 characters) or a key (`^#?[A-Z0-9][A-Z0-9_-]{0,39}$`, upper-cased); the branch is `<KEY>-<name>` (key without `#`, not repeated when the name starts with it), else the name; the repo is a git repo (its `origin` is fetched first, whether xherd cloned it or it is a local path; a failed fetch is only a warning, a repository without `origin` is left alone); profile resolved; plugins usable; flow exists in the selected pack (`none` requires a prompt); requested tool-permission mode exists in every runtime used, and `all` requires the sandbox flag; job name, branch and worktree path are free; base branch resolves (profile → repo default; a plain name in a clone resolves to `origin/<name>`; a plain local base branch is fast-forwarded to `origin/<name>` before the worktree is created when that is possible, and left as it is when it is ahead, diverged, dirty or checked out elsewhere). Any failure returns one clear error and changes nothing. |
| R-002a | **Tool-permission mode** per job (`ask`, `edits`, `auto`, `all`; §4.1 of the design): from the request, else the repo profile, else the global setting (default `edits`). It can be changed on a live job; the new mode applies from the next session start (fresh session, resume, restart), since the runtime takes it at launch. `all` is offered and accepted only in the container. |
| R-003 | A valid job is saved as `queued` and started when a slot is free (§5.3). |

### 5.2 Launching

| ID | Requirement |
|---|---|
| R-004 | Create the worktree with `git worktree add --no-track -b <name> <path> <base>` (no upstream, so a plain push cannot land on the base branch). |
| R-005a | Copy or symlink the profile's untracked files (e.g. `.env`) into the worktree. |
| R-005b | Install the runtime's status reporting (e.g. Claude Code hook entries in `.claude/settings.local.json`, tagged so they can be replaced; refuse if the existing JSON is invalid) and add every xherd-written path to `.git/info/exclude`. |
| R-005c | Write `.xherd/job.json` (`{id, name, branch, ticket, pack, flow}`) and `.xherd/instructions.md` (§16.5). |
| R-005d | Resolve the toolchain for the worktree (§12.4) in its own tmux window, then start the runtime in a tmux session named after the job, with `XHERD_ENV=1`, `XHERD_BIN`, `XHERD_JOB_ID`, `XHERD_JOB_TOKEN`, `XHERD_PORT_BASE` in its environment, and the flow's start command as the first prompt. |

### 5.3 Lifecycle

Statuses: `queued`, `running`, `waiting-input`, `blocked`, `idle`, `parked`, `paused`, `watching`, `done`, `cancelled`, `error`.

| From | To | When |
|---|---|---|
| queued | running | A slot is free (live jobs < `maxConcurrentAgents`, default 4) |
| running | idle | The job stopped and nothing waits on the user |
| running | waiting-input | The job stopped at a gate or asked a question |
| running | blocked | The terminal shows a dialog (permission prompt) or the job reported `blocked` |
| blocked | running | Work resumed / dialog gone |
| idle, waiting-input | running | User message, approval or change request |
| waiting-input | parked | Waiting longer than `gateParkMinutes` (default 30; 0 = never) |
| parked | queued | Approve or message (needs a slot again) |
| running | paused | User paused (handoff written, session ended) |
| paused, cancelled, error, watching | queued | Resume |
| idle | watching | PR open and nothing left to do (a stop in the ship or a PR stage, or in implement with every task done) |
| watching | queued | New PR review comments |
| watching | done | PR merged or closed |
| running | error | The terminal session disappeared |
| any live | cancelled | User cancelled |

| ID | Requirement |
|---|---|
| R-006 | **Live** statuses (`running`, `waiting-input`, `blocked`, `idle`) count against the cap. TODO checks do not. |
| R-007 | **Needs you** = `waiting-input`, `blocked`, or an open gate. |
| R-008 | Reports from a session xherd already ended (paused, queued, cancelled) never revive the job. |
| R-009 | Every status report carries a per-job **monotonic sequence number**; reports with a lower or equal number are logged but do not change state. Reports synthesised by the daemon (screen rules) get daemon sequence numbers and never make a real report stale. |
| R-010 | Every change to a job goes through one serialised path per job; no change is lost when reports, user actions and timers arrive together. |
| R-011 | **Reconcile** on start and every minute: a live job whose tmux session is gone → `error` (event `session-lost`). At daemon start (a container or terminal-server restart) such a job is instead parked when it waits at a docs or commit gate (approving still works), or resumed like R-027. A session that vanishes while the daemon runs is resumed by itself at most twice per hour per job (event `auto-resume`, with a notification); after that the job stays in `error` until the user presses Resume. |

### 5.4 Status reporting

| ID | Requirement |
|---|---|
| R-012 | Runtimes with hooks call `xherd hook <event>` (hook JSON on stdin). The command must never print to stdout or exit non-zero, because runtimes may show hook output to the model. |
| R-013 | Any job or wrapper can push status with `xherd report --state working|idle|blocked [--message] [--session] [--resume-cmd] --seq N`, and `xherd report --release` on exit. This is the contract for third-party jobs ("add xherd support to your job"). |
| R-014 | The CLI finds its job through `XHERD_JOB_ID` or `.xherd/job.json` (older sessions: `XHERD_AGENT_ID`, `agent.json`) in the current directory or a parent, and posts to the daemon. If the daemon is down, it appends to `hook-queue.jsonl`, replayed on daemon start in order and de-duplicated by event id. |
| R-015 | Event effects: session start → `running`; prompt submitted → `running`, acknowledges delivery, records "user prompted" when it is not a slash command; tool use / `working` → `running` (not logged; frequent); dialog notification → `blocked` + notify; stop / `idle` → `idle` or `waiting-input` (gate open or the last message asks a question), records running background tasks, triggers token usage, auto-approval, PR lookup, parking. |
| R-016 | Each report also updates the runtime session id, token usage, and last activity time, and interrupts a running auto-approval check when new work starts. |
| R-017 | **Reply detection**: at a gate, if the job's last message asks something, or it stopped without changing the gate documents after the user's message, the UI shows the job's reply and asks for the user's answer. |
| R-018 | **Screen rules** for runtimes whose dialogs have no hook: every 3 s, the last N lines of the pane are matched against the runtime's patterns; a match reports `blocked`, its disappearance `working`. Real reports win. |
| R-019 | **Idle alert**: a job idle longer than `idleAlertMinutes` (default 15) notifies once. |

### 5.5 Sending text to jobs (delivery)

| ID | Requirement |
|---|---|
| R-020 | Terminal driver: text is typed into the job's window, then Enter (runtimes may need extra Enters after slash commands). ACP driver: an ACP prompt message. |
| R-021 | xherd never types into a `blocked` job. |
| R-022 | Delivery is acknowledged by the next prompt-submitted or `working` report within a timeout (default 10 s); otherwise Enter is retried once and the user is notified. |
| R-023 | **Hand-on** after an approved gate: with fresh sessions on, only the job window's process is replaced (`tmux respawn-window`) by a new runtime session started with the next command, so other windows (a dev server, tests) keep running; the new session knows nothing of the last one and reads `memory.md` (state, decisions, code map) and the approved documents, not the codebase; otherwise the command is typed into the current session; a parked or watching job is started with it. |
| R-024 | No fresh session while the job has background tasks running (they would be killed), or for flow `none`. |
| R-025 | A change of runtime between steps always uses a fresh session in the new runtime (status files and permissions for that runtime are written first). If the new runtime is unusable, the job stays and the user is warned. |

### 5.6 Pause, resume, park, cancel, remove

| ID | Requirement |
|---|---|
| R-026 | **Pause**: ask the job to write a handoff (pack `handoff` command), wait for it to stop (≤ `pauseTimeoutSec`, default 120), end the session, status `paused`. Flow `none`: end the session. |
| R-027 | **Resume** (the Resume button, also for a job in `error`): back into the same runtime session when the runtime can resume it (the plugin's `data.terminal.resume`, used only while the runtime still has that session's transcript, and not twice in a row without a `session-start` in between), otherwise (or when a handoff was written and fresh sessions are on) a fresh session with the pack's `resume` command. A pending "next prompt" set by xherd (e.g. PR review) takes precedence. |
| R-028 | **Park** (automatic, §5.3): end the session; the job stays in Needs you. A message resumes the same session; approval starts a fresh one for the next step. |
| R-029 | **Cancel**: end the session without handoff; keep the worktree; resumable. |
| R-030 | **Remove worktree** (done or cancelled jobs only): archive `.xherd/` to `<data>/archive/<repo>/<job>/`, `git worktree remove`, keep the branch. |
| R-031 | **Delete**: remove the job's records (not the branch). |
| R-032 | **Pause all** (UI and `xherd pause --all`): a kill switch that pauses every live job at once (§13). |

### 5.7 No-workflow and folder jobs

| ID | Requirement |
|---|---|
| R-033 | Flow `none`: the prompt is sent as is; no gates, tasks or handoff. Status, pause/resume, PR follow-up (notify only) still work. |
| R-034 | Workspace `folder`: the job runs in `<data>/scratch/<name>/` (or a chosen folder) without a branch; git-dependent features are hidden. Instructions tell it which repo it may read. |

### 5.8 Saved agents

An **agent** is a saved, versioned setup that jobs start from (issue #141); a **job** is one run. (What older text called an agent is a job.)

| ID | Requirement |
|---|---|
| R-035 | An agent has an id, a slug name (unique, fixed), a description, a version (1 on creation, +1 on each edit, every version kept), runtime, model per runtime, autonomy, tool-permission mode, instructions, a job token budget, guardrails (`sensitivePaths`, `maxDiffLines`), skills, `maxLiveJobs`, and `archivedAt`. It has no repository, base branch or workflow: those belong to the job, the repository profile and the workflow's stage assignments (§5.9, R-041). Archived agents start no jobs. A job keeps a snapshot of its agents, so later edits never change a running job. |
| R-036 | Resolution order for a job: request, agent, repo profile, global settings. A profile's autonomy and permission mode are ceilings on all of them. An agent can only tighten guardrails; it cannot loosen a gate. Marking a stage `human` is part of its assignment (R-041), not of the agent. |
| R-037 | Agents are created, edited, archived, deleted, exported and imported (import refuses an existing name and lists risky settings first) from the UI, the CLI and the HTTP API. Status, live jobs, last job and tokens today are derived from jobs. |
| R-038 | The skills library (`<data>/skills/<name>/`, a `SKILL.md` per skill) is copied into a job's worktree where the runtime plugin declares a skill folder; a skill the repository already has is skipped and said so. |
| R-039 | An agent has a memory (at most 4 000 characters) copied into the job's `instructions.md`. A job may propose an update (`xherd agent memory propose`); the user accepts, optionally edited, or dismisses it. |
| R-040 | A job without an agent may be suggested as one when it is done, marked done (flow `none`) or is the third with the same setup in 30 days; a job may also draft the agent (`xherd agent propose`). Never after cancel or error; switched off by `suggestAgents`. |

### 5.9 Agents on workflow stages (accepted)

Issue #141 (second part, D26). An agent is a **persona**: who does the work. A **workflow** says which stage is done by which agent, so one job can have an architect for spec and design, a coder for implementation and a reviewer for the review. What a job works on (repository, base branch, workflow) belongs to the job, the repository profile and the request, never to the agent.

| ID | Requirement |
|---|---|
| R-041 | A **stage assignment** names, for a workflow (pack + flow) and one of its stages, a saved agent and optionally `human` (the stage's gate is decided by a person whatever the autonomy). Assignments are kept by the daemon (not in the pack, which cannot know the user's agents), edited on the Flows page, the CLI and the HTTP API, and apply to every job of that workflow created afterwards. Built-in and plugin packs are listed there; the pack itself is read-only. |
| R-042 | A stage with no assignment is done by the agent that ran the job's previous stage; the first stage with none uses the agent given at creation (`--agent`, meaning every stage not assigned), else no agent: the job behaves as one without agents. A job request may override the assignments for that job (`stage → agent`). |
| R-043 | At creation the job keeps a snapshot of every agent its workflow uses (the assignments, the request's overrides and the default), each at the version current then. Later edits to agents or assignments never change a running job. A job whose snapshot has an archived or broken agent is refused (`agent_archived`, `agent_broken`). Archiving or deleting an agent that an assignment names is refused (`agent_assigned`) until the assignment changes. |
| R-044 | When a stage starts, the job's settings for it are those of the stage's agent: instructions, memory, skills, tool-permission mode, model, autonomy and guardrails (R-036 applies per stage). The previous agent's skills that xherd copied are removed and the new agent's copied, and `instructions.md` is rewritten. A stage may start with another model or mode than the one before, always in a fresh session (D23). |
| R-045 | All agents of one job use the same runtime until R-025 is built: creating a job, or saving an assignment, that mixes runtimes is refused (`mixed_runtime`). Once R-025 exists the restriction is lifted without changing the records. |
| R-046 | Budgets, time and live-job limits follow the agent running the job's current stage: tokens and time are booked to it, and `maxLiveJobs` counts a job against the agent of its current stage when it is spawned. The job's token budget is the request's, else the lowest `jobTokenBudget` among its agents, else the profile's, then the global one. |
| R-047 | An agent memory proposal (R-039) made during a stage belongs to the agent of that stage. A job that has any agent is never suggested as one (R-040). |
| R-048 | An agent may name one `fallbackRuntime` (its model is `models[<fallback>]`). It is used, once, when the agent's runtime is unusable at job creation (plugin missing, off, no terminal driver), and recorded in the job's snapshot (`fellBack`); it is never switched while the job runs (R-025) and a failing session does not trigger it. An agent is broken only when neither runtime is usable. |
| R-049 | The Herd shows jobs only: List and By stage. There is no By agent view; what each agent is doing is on the Agents page (live jobs against `maxLiveJobs`, jobs, tokens today). Board cards name the agent of their stage. The view plugin protocol is unchanged. |

## 6. Gates, approvals and autonomy

### 6.1 Gates

| ID | Requirement |
|---|---|
| R-101 | Gate kinds: `docs` (one or more documents of a stage), `commit` (one proposed commit), `pr-triage` (new PR comments analysed), `pr-replies` (drafted replies), `question`. |
| R-102 | Gates are **declared** by the job through XAP (`xherd gate open …`, `xherd commit propose …`, `xherd ask …`) and validated against the files. If a job stops with an unapproved gate document but declared nothing, xherd infers the gate from the files and logs `gate-inferred`. |
| R-103 | Priority when several apply: commit > PR triage/replies > documents > question. |
| R-104 | **Approve**: append an approval to the ledger (R-106), snapshot the reviewed text for "changed since you reviewed", log the event, and hand on the next command (§5.5). |
| R-105 | **Request changes**: send the job a message naming the gate, the files and the user's comment, asking it to revise and stop at the gate again. |
| R-105a | **Open questions** of a documents gate come from the gate documents' `## Open questions` section (one list item per question; "None." or an empty section = none), not from a separate list the job passes. The user answers each one with an inline comment on its list item: the comment boxes of those items open by themselves when the gate opens, and the answers are sent together with the other inline comments as one change request (R-105). A question asked outside the documents (`xherd ask` while the gate is open, or a `--question` that matches no list item) gets its own comment box above the documents. Unanswered questions stay open (R-113). |

### 6.2 Approval by content hash

| ID | Requirement |
|---|---|
| R-106 | Every approval (human or automatic) appends `{gate, file, sha256, actor, auto, reason, at, commit}` to the job's approval ledger and the daemon database. The daemon **never edits** the job's documents, except the progress lines it owns: task `status` / `evidence` in the tasks document and commit `status` / `sha` in `commits.md`, written after the XAP call that changes them. |
| R-107 | A document is approved when the ledger has an approval whose hash equals its current content (line endings normalised). Editing an approved document re-opens its gate ("changed since approved") and the UI shows the diff against the approved version. Exception: in the tasks document (the one the flow's `tasks` stage gates), task progress lines (`- status:` and `- evidence:` under a task heading) are not content; recording progress does not re-open the gate. |
| R-108 | Several documents changed together form **one** gate approved together. |
| R-108a | **Commit gates**: the coding agent proposes the files and the full commit message (`commit propose --message`). The Changes view shows the diff of exactly those files and the user may edit the message. On approval **xherd makes the commit** (stages exactly the proposed files, commits with the message, hooks included, never `--no-verify`) and records the sha. If git refuses, the approval stands and the coding agent fixes and commits with git's output. The Commits view lists every commit with its message and files; the Changes view has tabs for uncommitted changes, all changes against the base, and any one commit. |

### 6.3 Autonomy

Each stage has an **autonomy class**. The level decides which classes xherd may approve:

| Class | `ask` | `important` | `never` | xherd stages |
|---|---|---|---|---|
| `critical` | wait | wait | approve | spec, ship |
| `routine` | wait | approve | approve | design, tasks, review |
| `code` | wait | approve | approve | commit |
| `human-only` | wait | wait | wait | PR triage, PR replies |

| ID | Requirement |
|---|---|
| R-109 | Autonomy is set per job (default from profile, then Settings) and can change live; `instructions.md` is rewritten when it does. |
| R-110 | Auto-approval runs when the job stops at an allowed gate: mark "checking", evaluate tripwires (some run commands), and approve only if the gate is unchanged and the coding agent did not ask anything. A user message in the meantime cancels the check; it re-runs at the next stop. |
| R-111 | Each auto-approval is listed in the **Inbox** (*Updates*) with the reason, the files, and the job's stated assumptions, and notifies the user. |

### 6.4 Tripwires

A gate waits for a human, with the reason shown, when any of these hold:

| ID | Tripwire |
|---|---|
| R-112 | A document or commit is marked `needs-human: yes` (with a reason). |
| R-113 | The spec has open questions (when approving the spec). |
| R-114 | Review or ship: the review has blocker/major findings, or review-sourced tasks are not done. |
| R-115 | Code gates: changed files match the profile's **sensitive paths** (globs); the diff exceeds `maxDiffLines` (default 1000) or adds a file too large to count; the profile's **test command fails**. |
| R-116 | Commit gate: staged files differ from the task's declared `files:` (**scope fence**), except the job's own spec folder in committed mode. |
| R-117 | Commit gate: the diff contains something that looks like a **secret** (built-in patterns for common token formats, private keys, `.env` content). |
| R-118 | A task's `verify:` command fails. |
| R-119 | A blocking **stage hook** failed (§8.5). |
| R-120 | **Run limits** reached: more than `maxAutoApprovalsInRow` (default 10) consecutive auto-approvals, more than `maxReviewLoops` (default 3) review→fix rounds, or the job's token **budget** is used up. |
| R-121 | (Dropped: no `hotfix` flow. A hotfix is a `bugfix` job on another base, and ship is `critical` in every flow.) |

## 7. xherd: the built-in SDD

xherd is xherd's own SDD, written for xherd: documents for each step, approval gates, one commit per task, review in a fresh context, and PR rounds. Its prompts and formats ship as the built-in pack `xherd`; their exact text lives in `packs/xherd/` with `FORMAT.md` and JSON schemas (to be written).

### 7.1 Flows

- **feature**: spec ✋ → design ✋ (skippable) → tasks ✋ → implement (commit ✋ per task) → review ✋ → ship ✋ → PR rounds → done.
- **bugfix**: analysis (investigation, root cause, fix approach, the reproduction test; `spec.md` and `tasks.md` under one gate) ✋ → reproduce (the failing test alone, its failing run as evidence; commit ✋) → fix (commit ✋ per task, each verified by the reproduction command) → review ✋ → ship ✋ → PR rounds → done.
- **PR rounds**: new comments are analysed into items (triage ✋, a decision per comment), fixes are implemented with commit gates, replies are drafted (replies ✋) and posted.

Design may be skipped only when the change is small and local, has no data/API/config/dependency changes, touches nothing security- or concurrency-sensitive, and has one obvious approach. The spec states the decision; approving the spec approves the skip.

### 7.2 Commands

| Command | Does | XAP calls |
|---|---|---|
| `/xherd:feature <KEY> <prompt>` | Read the ticket, explore, write `ticket.md` and `spec.md` | `gate open --role spec` |
| `/xherd:bugfix <KEY> <prompt>` | Investigate without changing repository files, write the bug `spec.md` and `tasks.md` (T01 = the reproduction test) | `tasks sync`, `gate open analysis` |
| `/xherd:reproduce` | Write the reproduction test only, see it fail for the analysed reason, propose its commit | `task start/done`, `commit propose` |
| `/xherd:design` | Write `design.md` | `gate open --role design` |
| `/xherd:tasks` | Write `tasks.md` | `tasks sync`, `gate open --role tasks` |
| `/xherd:next` | Implement the next ready task, verify, propose a commit; when all are done, start the review | `task start/done`, `commit propose` |
| `/xherd:commit <Cnn>` | Commit exactly the proposed files, continue | `commit done` |
| `/xherd:review` | Review the branch in a fresh context; findings become tasks; propose memory updates | `gate open --role review` |
| `/xherd:ship` | Draft `pr.md` following the repo's PR template; after approval push and open a **draft** PR (the coding agent does not touch the ticket: xherd offers the user a summary once the PR is open, R-308) | `gate open --role ship`, `pr opened` |
| `/xherd:pr-review` / `pr-apply` / `pr-fetch` / `pr-post` | PR rounds (fetch/post only for hosts without those plugin commands) | `gate open pr-triage / pr-replies` |
| `/xherd:handoff` / `resume` / `status` / `rules` | Handoff note; reload and continue; print state; shared rules | `handoff written` |

### 7.3 Documents

Documents live in the job's spec folder (§7.5): `ticket.md`, `spec.md`, `design.md`, `tasks.md`, `commits.md`, `review.md`, `pr.md`, `memory.md` (short state for fresh sessions), `pr-review.md`, plus private `HANDOFF.md` and `pr-comments.json`.

- Frontmatter carries `ticket`, `flow`, `stage`, optional `needs-human` + `reason`, `design: skip` (spec), `template` and `url` (pr). There is **no `approved` key**: approval lives in the ledger (§6.2).
- Gate documents end with the job's **assumptions**; the spec and the design also list **open questions** (none → "None."), the only place the job states them (R-105a).
- Tasks have stable ids (`T01`…), state (todo, doing, blocked, done), refs to requirements, `files:` (binding, R-116), `depends:`, `evidence:`, optional `verify:` command, optional model **tier** hint, optional **codebase contract** (verified file:line references and things that do not exist), and `source:` (spec, review, pr).
- Commits have ids (`C01`…), state (proposed, done, dropped), tasks, files, sha; one proposal at a time.

### 7.4 Additions

| ID | Requirement |
|---|---|
| R-131 | Discovered issues: `xherd issue add` records out-of-scope findings in the issue ledger; the UI lists them; at ship they can become follow-up tickets or TODOs. |
| R-132 | Second opinion: the review step can run on a different runtime/model than implement (ordinary per-step configuration, §10.3). Settings suggests it when one runtime is used everywhere. |
| R-133 | Task waves: tasks whose dependencies are all done form a wave, shown in the UI (parallel execution is roadmap). |
| R-134 | Taxonomy: optional `projects:` and `tags:` in spec frontmatter, used as filters and PR labels. |
| R-135 | Branch roles per profile: `baseBranch`, `stagingBranch`. |

### 7.5 Where documents live

**Default: private.** A repo may host tasks driven by different SDDs, so nothing an SDD writes is committed by default:

```
<worktree>/.xherd/                  # in .git/info/exclude
  job.json  instructions.md
  spec/       ticket.md spec.md design.md tasks.md commits.md review.md pr.md memory.md pr-review.md
  ledger/     approvals.jsonl issues.jsonl
  HANDOFF.md  pr-comments.json
```

| ID | Requirement |
|---|---|
| R-151 | By default SDD documents and ledgers live in `.xherd/` and are never committed. One job uses one pack, recorded in `job.json`. |
| R-152 | **The "why" travels in the PR**: at ship, the PR description carries the essentials (summary, requirements covered, key decisions, assumptions). |
| R-153 | **The repo's PR template comes first.** The host plugin's `pr-template` command returns the repo's templates (GitHub `.github/pull_request_template.md`, `.github/PULL_REQUEST_TEMPLATE/*.md`, `docs/`, root; GitLab `.gitlab/merge_request_templates/*.md`; organisation defaults via the API, **verify**). `pr.md` keeps every heading of the template, places the essentials in fitting sections, ticks checklist items only when true, and puts anything left over in a short `## Spec summary` at the end. With several templates the job picks one and names it in frontmatter. Only without a template does xherd use its own layout. Optional per profile: a collapsed full copy of spec and design. |
| R-154 | Archived `.xherd/` folders stay viewable in the UI after the worktree is removed; optional pruning after N days (off by default). |
| R-155 | **Opt-in committed mode** per profile (`specDir: xherd/{job}`): documents and ledgers are written there and committed with the job's normal commits; the scope fence allows the job's own folder and rejects other jobs' folders; private files stay in `.xherd/`. Folder names are job names, so parallel jobs never collide. xherd may suggest a `.gitattributes` line that collapses these files in PR diffs (never added silently). |

## 8. Pluggable SDD: flow packs, step roles, dynamic flows

### 8.1 Principle

The daemon knows **stages, gates, tasks, commits and PR rounds** in the abstract. A flow pack maps its own documents and commands onto them, and the coding agent declares each transition with XAP. The daemon validates each declaration and either accepts it or refuses with a message the coding agent can act on.

### 8.2 Flow packs

| ID | Requirement |
|---|---|
| R-201 | A flow pack is a directory or git repo with `xherd-flow.yaml` (schema in design §6). |
| R-202 | The manifest declares: name, version, XAP protocol version, flows, each flow's stages with **role**, gate, autonomy class and **next** command (with a small fixed set of conditions), and the `start`, `resume`, `handoff`, `status` commands. |
| R-203 | Install targets per runtime (paths, command prefix, format: command, skill, workflow, job). xherd records the hash of each installed file; upgrades replace unchanged files and keep files the user edited (writing the new version next to them and warning). The install runs at every daemon start (native and container), compares content so a current install writes nothing, lists recorded files the pack no longer ships as obsolete (removed only on the user's confirmation or `xherd install --remove-obsolete`), and lets the user answer "Keep mine" for an edited file (the declined version is not written again). Setup checks `pack` (missing or out of date) and `pack-files` (edited and obsolete files, with a review dialog) report it. |
| R-204 | Optional JSON schemas for document frontmatter, checked by `xherd gate open`. |
| R-205 | Optional `parse` command that turns the pack's files into the normalised state (R-213). Without it, the pack keeps state current through XAP calls or uses xherd formats. |
| R-206 | Docs location: private by default; a profile may opt in to a committed path; a pack that only works with committed files declares `requiresCommitted: true` and xherd warns when the repo has not opted in. |
| R-207 | `xherd flow test <dir>`: manifest schema, prompt rendering per runtime, parser output on fixtures, and a fake-runtime run of each flow through its XAP calls. |
| R-208 | Conformance levels unlock UI features: L0 status only, L1 gates, L2 tasks + commits, L3 PR rounds, L4 stage hooks + ledgers. |
| R-209 | A pack never needs daemon code changes; missing capabilities mean a new XAP version. |

### 8.3 Step roles

| ID | Requirement |
|---|---|
| R-210 | Canonical roles: `explore`, `spec`, `design`, `tasks`, `implement`, `review`, `ship`, `pr`. Each stage declares its role. Per-step settings (runtime, model/tier, effort, driver), autonomy defaults, Kanban stage cards and icons are keyed by role, so a setting such as "review on Codex" applies to every pack. |
| R-211 | Several stages may share a role. A stage without a fitting role uses `custom` and declares its class; per-step settings fall back to the job's default. |
| R-212 | `xherd flow test` fails when a stage has no role and warns when a pack lacks `spec`, `implement` or `ship`. |

### 8.4 Normalised state

| ID | Requirement |
|---|---|
| R-213 | Whatever the pack, the daemon and UI work with: current stage; stages with status (pending, active, gated, approved, skipped); the gate; tasks; commits; PR items; open questions; issues; warnings; and free-form `extra.*` data from the pack's parser for custom views (§11). |

### 8.5 Stage hooks

| ID | Requirement |
|---|---|
| R-401 | Profiles and packs may declare hooks per role and event: `pre.<role>`, `post.<role>`, `gate.<role>`, `pr.opened`, `pr.merged`, `job.created`. |
| R-402 | Hook kinds: `command` (argv in the worktree, timeout, `XHERD_*` env), `agent` (a one-shot read-only prompt in the job's runtime), `builtin` (`notify`, `tracker.transition`, `label`). |
| R-403 | `blocking: true` on `pre.<role>` makes the hook a tripwire for that stage's gate; declared artifacts (globs) are attached to the gate. |
| R-404 | Hook output goes to the job log with secrets redacted; non-blocking failures only notify. |

```yaml
stageHooks:
  pre.ship:
    - { name: e2e, kind: command, run: ["pnpm", "exec", "playwright", "test"], blocking: true, timeoutSec: 1200, artifacts: "test-results/**/*.webm" }
  post.pr:
    - { name: jira-in-review, kind: builtin, builtin: tracker.transition, with: { status: "In Review" } }
```

### 8.6 "Add xherd support to your SDD" (dynamic flows)

SDD authors add a few `xherd` calls to their own commands; the SDD then works under xherd **without a pack**, and keeps working standalone (the calls are skipped when `XHERD_ENV` is not `1`).

| Level | SDD author does | User gets |
|---|---|---|
| 0 | Nothing | Flow `none`: status, terminal, PR follow-up |
| 1 | Adds XAP calls guarded by `XHERD_ENV=1` | Gates, approvals, autonomy, per-step runtimes; stages appear as they are declared |
| 2 | Ships `xherd-flow.yaml` | Stage list up front, schemas, parser, views |
| 3 | Lets xherd install its prompts | `xherd install` sets it up in every runtime |

| ID | Requirement |
|---|---|
| R-221 | Sessions run with `XHERD_ENV=1`, `XHERD_BIN`, `XHERD_JOB_ID`, `XHERD_JOB_TOKEN`. |
| R-222 | Registering a dynamic flow needs a name, the start command template (`/sdd-spec {prompt}`) and the allowed command prefixes (`/sdd-`). |
| R-223 | `gate open … --next "<command>"` tells xherd what to send after approval; it must start with an allowed prefix. Without it xherd sends the resume command or asks the user. |
| R-224 | `--role` maps the SDD's stage to a step role; missing → `custom`, class `critical` (always waits) until the user changes it. |
| R-225 | Optional calls (tasks, commits, PR, handoff, issues) can be adopted one at a time. |
| R-226 | Calls never break the SDD: exit 0 ok, 2 refused (fix and retry), 3 daemon unreachable (stop and wait); report-only calls queue offline. |
| R-227 | xherd ships `doc/integrate-your-sdd.md` with snippets for prompt-based and script-based SDDs. |
| R-228 | `xherd sdd check <dir>` reports the level an SDD reaches; `xherd sdd try <dir>` runs a fake job through its calls. |
| R-229 | Compatible SDDs use the GitHub topic `xherd-sdd` and appear in the catalogue. |

Example block for a prompt-based SDD command:

```markdown
## When running under xherd
If the environment variable `XHERD_ENV` is `1`, after writing the spec run
`xherd gate open spec --role spec --doc <spec path> --next "/sdd-task <spec path>"`
then stop and wait. If it exits with code 2, fix what it says and run it again.
```

## 9. Pull-request follow-up

| ID | Requirement |
|---|---|
| R-301 | After ship, the job's session ends; status `watching` when the PR is open and nothing is left to do. |
| R-302 | Without a declared PR URL, the host plugin's `pr-for-branch` finds it; for flow `none` the user may paste a link. |
| R-303 | Every `prPollMinutes` (default 5): PR status (state, number, mergeable, reviewers), checks, and comments through host plugin commands. |
| R-304 | New comments (ids not seen, not written by xherd, recognised by a hidden marker in the body) are saved to `pr-comments.json` and the job is queued with the PR review command (fresh session unless off). Flow `none`: notify only. |
| R-305 | CI: notify when checks start failing and when all pass again. Merge conflicts: notify once. |
| R-306 | PR decisions: the user decides resolve/skip (with a note) per comment; xherd writes the decisions and sends the apply command. Replies: approved replies are posted by the host plugin (with the marker), or by the job for hosts without a reply command. |
| R-307 | Merged or closed: follow-up task done, stage `done`, status `done`, `pr.merged` hooks run. |
| R-308 | Ticket updates happen only when the user accepts them. Once the pull request is open, xherd offers a short summary of the work (built from finished tasks, commits and the PR, independent of the pack), the user reads and edits it, and only their click posts it through the tracker plugin. `pr.md` is the pull request's body and holds nothing for the ticket. Status transitions and opt-in stage hooks are not built. |

## 10. Plugins

### 10.1 Kinds, manifest, protocol

| ID | Requirement |
|---|---|
| R-501 | Kinds: `runtime`, `tracker`, `host`, `flow`, `notifier`, `toolchain`, `panel`. |
| R-502 | Manifest `xherd-plugin.json` (packs: `xherd-flow.yaml`) with `name`, `kind`, `version`, `min_xherd_version`, `label`, `description`, `settings` (key, label, type, default, secret, required), kind-specific `data`, prompt `instructions` fragments, `commands`, `hooks`, `checks` (argv, fix, optional; pass = exit 0 within 10 s), `build` (install-time), `timeoutSeconds` (default 60). |
| R-503 | Commands are argv in any language (no shell); `./x` resolves inside the plugin folder. stdin carries `{command, input, settings, secrets, job}`; stdout returns JSON. Non-zero exit, timeout (process tree killed), bad JSON or wrong shape = failure, shown on the job and in Setup checks; stderr goes to the log with secrets redacted. |
| R-504 | Built-in plugins (GitHub, GitLab, Bitbucket; Claude Code, Codex, opencode; SDKMAN, nvm, uv, Go, rustup; xherd) are compiled into the binary behind the same interface; an external plugin with the same name overrides one. |
| R-505 | **The CLI is the plugin API**: commands run with `XHERD_BIN`, `XHERD_URL`, a plugin-scoped `XHERD_PLUGIN_TOKEN`, `XHERD_PLUGIN_ID`, `XHERD_PLUGIN_ROOT`, `XHERD_PLUGIN_CONFIG_DIR`, `XHERD_PLUGIN_STATE_DIR` and `XHERD_CONTEXT_JSON`, and may call any `xherd` command their token allows. No SDK. |
| R-506 | Settings resolution: manifest default → Settings → profile override (non-secret only). Secrets are global, stored in `secrets.json` (mode 0600), never sent to the UI (it only learns which are set), passed only on stdin to that plugin. |
| R-507 | A broken plugin is listed with its error but cannot be chosen. |

Host commands: `pr-for-branch {branch} → {url|null}`, `pr-status {url} → {state, number, url, mergeable?, reviewers?}`, `pr-checks {url} → [{name, bucket, link, workflow}]`, `pr-comments {url} → [{id, kind, author, body, url, createdAt, path?, line?, inReplyTo?}]`, `pr-reply {url, to, body} → {}`, `pr-template {} → [{path, name, body}]`. Runtime usage: a declarative `data.terminal.usage` (JSON pointers into the transcript lines and a name per standard counter `input`, `output`, `cacheWrite`, `cacheRead`; the transcript is found through `resume`), or, for runtimes whose transcripts are not JSON lines, a runtime command `usage {session, cwd, payload} → {input, output, cacheRead, cacheWrite, context}` (not built yet). Event hooks receive `{event, job, ticket, settings, secrets}`; output ignored.

### 10.2 Runtime drivers

| ID | Requirement |
|---|---|
| R-511 | A runtime plugin declares the drivers it supports: `terminal` (launch/resume argv, command prefix, Enters after slash commands, permission modes, hook files, trust files, screen rules) and optionally `acp` (job argv, session resume support). No `acp` entry = no ACP support; the UI shows a badge for runtimes that have it. |
| R-512 | The driver is chosen per job, per profile or globally (most specific wins); only supported drivers are offered. It applies at the next session start. |
| R-513 | `terminal`: live terminal in the browser; the user can type into it. |
| R-514 | `acp`: xherd is the ACP client; prompts are exact messages; the UI shows a transcript; permission requests become `blocked` with Allow / Allow always / Deny (also on phones); "Take over in terminal" resumes the session with the terminal driver where the runtime allows it. |

### 10.3 Models and effort per step

| ID | Requirement |
|---|---|
| R-521 | Runtimes list models with a tier (`high`, `medium`, `low`) and how to pass them; optionally effort levels. |
| R-522 | Per-step configuration = runtime + model or tier + effort + driver, for each step role, at job, profile and global level; unset values fall back to the next level, then the runtime default. |
| R-523 | Packs and tasks refer only to tiers, never model ids, so packs stay portable. |

### 10.4 Toolchain plugins

| ID | Requirement |
|---|---|
| R-531 | A toolchain plugin provides tools: version files it reads, list/install/resolve commands (resolve returns PATH entries and env), default versions, checks, and an optional bootstrap that installs the manager itself into the home volume. |
| R-532 | Built-in: SDKMAN (Java, Maven, Gradle, Kotlin), nvm (Node) + corepack, uv (Python), Go toolchain switching, rustup. Others (.NET, Ruby, Flutter…) are plugins. When two plugins claim a tool, the profile chooses. |

### 10.5 Installing and creating plugins

| ID | Requirement |
|---|---|
| R-541 | Settings → Plugins → Add: search the catalogue (GitHub topics `xherd-plugin`, `xherd-flow`, `xherd-sdd`, indexed periodically, several plugins per repo allowed, no central registry and no review), or paste a git or archive URL. |
| R-542 | Before installing, show the manifest: kind, version, source, the commands, hooks and build steps it will run, checks, settings and secrets requested, files it will install into runtimes. The user confirms. If the manifest changes between preview and install, the install aborts. |
| R-543 | Installs are pinned to a commit sha (or archive hash); updates are explicit and show the manifest diff. The old version stays until work using it has finished. |
| R-544 | Installing is allowed for the local client and paired devices with full rights, never phones; `XHERD_PLUGIN_INSTALL=off` disables it on servers. |
| R-545 | Plugins run with the rights of the user that runs the jobs, never with more. Natively that is the user's own; in the two-user container (R-1203) it is the agents' user, so a plugin can use the jobs' CLI logins (`gh`, `glab`, …) and cannot read the daemon's database, `config.json` or `secrets.json` (its own secrets reach it on stdin). External plugins are copied at daemon start to a folder the agents' user can read and run but not change, and their `XHERD_PLUGIN_CONFIG_DIR` / `XHERD_PLUGIN_STATE_DIR` are in the agents' home. There is no further sandbox in v1, and the install confirmation says so. |
| R-546 | A plugin used by a live job cannot be disabled or removed until the job is paused or done. |
| R-547 | `xherd plugin new --type <kind> --name <name> [--dir] [--lang rust|go|node|python|bash]` (interactive without flags) creates a working skeleton: manifest with all fields for the kind, one example command per protocol command returning valid output, README, fixtures, test config and a CI workflow. Right after, `xherd plugin test` passes and `xherd plugin link` makes it usable. `xherd flow new` scaffolds a pack (stages with roles, prompts per runtime, schemas, default views). |
| R-548 | CLI kit: `xherd plugin new|test|link|unlink|ls|search|install|update|remove`. `plugin test` checks a plugin's conformance (outputs validated; never runs reply/posting commands or hooks). |

## 11. Declarative views

Screens are built from **panels** arranged by declarations. xherd's own job page is declared the same way.

| ID | Requirement |
|---|---|
| R-1101 | Declarable: job page (header fields, gate texts per stage, rail sections and panels, actions), New job inputs (JSON Schema → generated form; values reach the job as `{{inputs.x}}`), Needs-you texts per gate, Kanban cards (record source, title, column mapping). |
| R-1102 | Built-in specific panels: `gate`, `documents`, `tasks`, `commits`, `diff`, `pr`, `terminal`, `transcript`, `log`, `issues`, `memory`, `toolchain`. |
| R-1103 | Built-in generic panels: `markdown`, `table`, `checklist`, `keyValue`, `timeline`, `gallery`, `media`, `files`, `progress`, reading from the normalised state, the pack's `extra.*`, or artifact globs. |
| R-1104 | Panels declare the capabilities they need (git, diff, pr, terminal…); unmet panels are hidden. Unknown types render a placeholder; `xherd flow test` validates views (`ui: 1`). |
| R-1105 | Actions are a fixed set: `approve`, `requestChanges`, `send` (message template), `runCommand`, `pluginCommand`, `openArtifact`, `openIn`; same API and permission rules as built-in buttons; `confirm:` for costly or destructive ones; only `visibleWhen` conditions, no scripting. |
| R-1106 | Panel plugins (post-v1): a web component in a sandboxed iframe on a separate origin, without the daemon token, talking to the page through `postMessage` (`getData`, `onData`, `requestAction`, `openArtifact`), receiving the UI's design tokens as CSS variables. |
| R-1107 | A `terminal` panel type runs a plugin's terminal program in a tmux window and shows it in the browser, so terminal tools can be plugins without web code. |
| R-1108 | A command palette (Ctrl/Cmd+K) lists built-in and plugin actions for the current context, with configurable shortcuts. |

## 12. Deployment

### 12.1 Docker-first

| ID | Requirement |
|---|---|
| R-601 | The recommended install is `docker compose up -d` with the published multi-arch image (amd64, arm64). One shared container runs the daemon and every job; it contains `xherd`, `git`, `tmux`, `gh`, `glab`, `openssh-client`, the agent CLIs and the version managers (§12.4). |
| R-602 | **Same-path workspace**: the workspace root (default `~/xherd` on the host) is mounted at the identical absolute path in the container, so repos, worktrees, `.git` pointers and paths shown in the UI are valid on both sides. |
| R-603 | The container runs jobs as the host user's UID/GID so files keep the user's ownership. |
| R-604 | Runtime logins, xherd data, toolchains and caches live in a named volume (`xherd-home`). |
| R-605 | Repos outside the workspace are rejected with the mount to add (`XHERD_EXTRA_PATHS`, also same-path) or a "clone into workspace" option. |
| R-606 | `xherd docker init` (host) writes the compose file and `.env` for the detected engine; `xherd docker doctor` checks same-path mounts, ownership, host-side file changes seen in the container, SSH job, and port binding to `127.0.0.1`, each with a fix. |
| R-607 | The native binary (host tmux) is supported with the same behaviour. |
| R-608 | Each job gets `XHERD_PORT_BASE` (a free block of 10 ports) so dev servers of parallel jobs do not collide. |

### 12.2 Container engines on macOS

Supported: Docker Desktop (reference), and OrbStack, Colima, Podman Desktop, Rancher Desktop when `xherd docker doctor` passes. **Experimental**: [Kiri](https://runkiri.com/) (Docker-compatible runtime, beta) and Apple's `container` with [opossum](https://github.com/suruseas/opossum) (Compose subset, macOS 26+, no Docker API). xherd does not install an engine.

### 12.3 Git access: HTTPS and SSH

| ID | Requirement |
|---|---|
| R-681 | Remote URLs are used as they are; one install may mix HTTPS and SSH per host. |
| R-682 | SSH: the host's SSH job forwarded into the container (Linux `$SSH_AUTH_SOCK`, Docker Desktop `/run/host-services/ssh-auth.sock`, exposed to the container user through a proxy owned by that user), or mounted/secret key files referenced in an xherd-managed SSH config include. |
| R-683 | Host keys: GitHub, GitLab and Bitbucket keys pre-seeded; other hosts' fingerprints shown in the UI for the user to accept. |
| R-684 | HTTPS: tokens through a git credential helper (`gh`, `glab`, or `xherd credential`), never in URLs or `.git/config`; optional CA bundle per host. |
| R-685 | Settings → Git access per host: protocols, credential source, username, CA; optional `preferHttps`/`preferSsh` rewrite (off by default). Host plugins reuse the token. |
| R-686 | Welcome steps "Commit identity" (name and email jobs commit as; required, git refuses to commit without it) and "Git hosts": use the SSH job, log in with GitHub's device flow (`gh auth login --web`, code shown in the UI), or paste a token; each ends with a live test of git transport and API access. |
| R-687 | `xherd git doctor [host] [--repo URL]` checks, in order: job socket readable, key loaded (`ssh-add -l`), host key known, credential helper configured, token valid with the needed permissions, organisation SSO authorised, OAuth app allowed by the organisation, git vs API access separately. It prints the first failure with its fix and never prints secrets. |
| R-688 | An auth failure during a job's push or a host command blocks the job with reason "git access" and shows the doctor result. |
| R-689 | **Providers and hosts (proposed, revises D19).** A *provider* is a `host` plugin (GitHub, GitLab, Gitea, Bitbucket, …): it owns the host's API (pull requests, checks, comments, replies, templates) and what is specific to its access. A *host* is configuration: a hostname bound to one provider, with its protocols, credential source, username and CA. Several hosts may share a provider (`gitlab.com` and `git.example.com` both use `gitlab`). The provider of a known hostname is filled in by itself; any other hostname is bound by choosing a provider. Issues are not a provider's job: they come from `tracker` plugins. |
| R-690 | A provider's manifest declares, in a `data.git` block: the hostnames it claims (patterns), the git credential helper (argv) and the token's username, the browser login (argv with `{host}`; absent = paste a token), the ordered doctor checks of its API (each with its fix), and the token permissions it needs (shown in the UI). Transport stays in xherd and per hostname: SSH job and keys, known hosts, token storage, the managed gitconfig, `preferHttps`/`preferSsh` rewrites, the pre-push guard. |
| R-691 | A provider that needs a command-line tool declares `requires` (the binary and its check), `install` (argv) and `instructions` (text per OS). In the container the install runs from the UI in a terminal, into the agents' home; natively xherd never installs on the host and shows the instructions (as R-653). The card, Setup checks and the Git hosts step say what is missing. A provider that talks to the API with a token needs none of this. |
| R-692 | Every argv a provider supplies is fixed; `{host}` and other values reach commands only as whole arguments, never spliced into shell text. An external provider is installed through the D16 flow, which shows the commands (credential helper, login, install) before they can run. |
| R-693 | A `config.json` written before providers keeps loading: `credential` `gh` binds the host to `github`, `glab` to `gitlab`, `xherd` and `none` to no provider (a host with no provider named is still bound to the first provider whose patterns claim its name, so API checks and the token user are kept). |

### 12.4 Toolchains in the shared container

| ID | Requirement |
|---|---|
| R-651 | The image ships version managers, not language runtimes (except what the agent CLIs need). Languages are installed into `xherd-home`. |
| R-652 | Before a job starts, xherd reads the worktree's version files (`.sdkmanrc`, `.nvmrc`, `.node-version`, `package.json` (`packageManager`), `.bun-version`, `.python-version`, `pyproject.toml`, `go.mod`, `rust-toolchain.toml`), installs missing versions with the matching toolchain plugin, and starts the runtime with that `PATH`. Each worktree gets its own versions; no global "current version" is switched. |
| R-653 | Welcome step "Toolchains" and Settings → Toolchains: languages with version pickers (defaults Java 21 + Maven + Gradle, Node LTS + pnpm, Python 3.12 + uv, Go, Rust stable), install with a live log, skip allowed. Native installs only detect, never install on the host. |
| R-654 | Profiles may declare `toolchain:` versions for repos without version files and `setup:` commands run once per new worktree. |
| R-655 | Setup checks show whether each profile's toolchain resolves, with "Install missing". |

### 12.5 Deployment modes

| Feature | Local Docker | Native | Server (Docker) | Kubernetes |
|---|---|---|---|---|
| Jobs, gates, UI, PR follow-up, phone | ✓ | ✓ | ✓ | ✓ |
| Web terminal; `xherd attach` over websocket | ✓ | ✓ | ✓ | ✓ |
| VS Code in the browser | ✓ | ✓ | ✓ | ✓ |
| Local apps via URL schemes | ✓ | ✓ | – | – |
| VS Code Remote-SSH / JetBrains Gateway | – | – | ✓ (SSH to host) | via SSH sidecar (**verify**) |
| Dev Containers attach | ✓ | – | via Docker context over SSH (**verify**) | – |
| Terminal on the user's computer (link handler) | ✓ | ✓ | – | – |

| ID | Requirement |
|---|---|
| R-671 | The UI offers only the Open-in entries that work in the current mode and client. |
| R-672 | Remote modes require HTTPS and device pairing; a Compose + Caddy variant and a Helm chart (one pod per user, PVCs for workspace and home, `Recreate` strategy) are shipped. |
| R-673 | Remote credentials come from secrets; the Welcome screen asks for them. |

### 12.6 Open in…

| ID | Requirement |
|---|---|
| R-701 | URL-scheme entries (no host software): `vscode://file/{dir}`, `cursor://file/{dir}`, JetBrains Toolbox and Zed URLs (**verify** formats). |
| R-702 | Dev Containers entry: open the worktree inside the xherd container (`vscode://vscode-remote/attached-container+<hex>{dir}`). |
| R-703 | Host companion (`xherd` on the host, client mode): `xherd attach <job>` (over the daemon's terminal websocket), `xherd open <job> [--app NAME]`, `xherd shell <job>`, `xherd login [URL]`. |
| R-704 | *Removed* (issue #129): the host bridge `xherd companion` is gone. Every Open-in entry is a link the browser opens; the terminal is opened by the link handler of R-707. |
| R-705 | VS Code in the browser (openvscode-server) inside the container, for remote devices. |
| R-706 | Settings → Open in manages the entries: show or hide each one (links, the terminal) and add **user links** (`openIn.links`: a label and a URL with `{dir}` and `{job}`, replaced percent-encoded). User links are for local apps only: the URL must start with a scheme, and web and browser-internal schemes (`http`, `https`, `ws`, `wss`, `ftp`, `javascript`, `data`, `vbscript`, `blob`, `file`, `about`, `xherd-term`) are refused. The user commands of earlier versions (`openIn.commands`) are dropped by a config migration and named once in Settings (`openIn.droppedCommands`) until dismissed. |
| R-707 | **Open in terminal** opens the user's own terminal app in the job's worktree, on the user's computer, through the link `xherd-term://open?dir=<dir>&terminal=<id>`. A handler script (`xherd-open-terminal`, POSIX `sh`, nothing kept running) is registered once for that scheme, for the current user: natively by `xherd terminal-handler install` or the Install button, in Docker by `curl -fsSL <origin>/api/v1/open-in/terminal-handler.sh \| sh` on the host (no `xherd` needed there). The handler opens only a folder that is itself a job worktree (`<worktrees root>/<repo>/<job>`, also after resolving symlinks, with a `.git` file and `.xherd/`), never runs anything taken from the URL, and uses the chosen terminal (macOS Terminal, iTerm or Ghostty; on Linux the desktop's default among the known ones, else `x-terminal-emulator`) with the folder as its working directory. The install reports to the daemon, which lists the Terminal entry only while it has a recorded install and offers an update when the handler it ships is newer or the worktrees root moved. macOS is checked by dry run only (**verify**). |

## 13. Guardrails and security

### 13.1 What is protected

The host machine; git and model credentials; plugin secrets; the integrity of repos (base and protected branches); the user's money (tokens, paid APIs); and the user's attention (no silent actions in their name).

### 13.2 Threats and guardrails

| # | Threat | Guardrails |
|---|---|---|
| T1 | **Prompt injection** from tickets, PR comments, web pages or repo files steering a job to run harmful commands, leak secrets or push bad code | Untrusted text is passed to jobs inside clearly marked blocks with an instruction to treat it as data (R-1201); PR triage and replies always wait for a human; autonomy limits and tripwires (§6.4); tool permissions: file edits in the worktree are accepted by default and everything else asks (`ask` mode is always available); secret tripwire on commits (R-117); egress allowlist (roadmap). |
| T2 | **Job approves its own gates** or changes xherd settings | XAP has no approve command; jobs get a job-scoped token valid only for reports and XAP of that job (R-1202); in Docker, job processes cannot read the daemon's config and secrets (R-1203). |
| T3 | **Pushing where it must not** (base branch, force push, other branches) | Worktrees have no upstream; an xherd-managed `pre-push` hook in each worktree rejects pushes to any ref except the job's branch and any non-fast-forward push (R-1204); xherd rules forbid force pushes; the docs recommend branch protection on the host. |
| T4 | **Destructive commands** in the workspace (deleting repos, `git reset --hard` on the main checkout) | Jobs work only in their worktree; tool permission `all` only inside the container; the container protects the host but not the workspace mount, so backups (§15) cover the workspace's unpushed work; the kill switch (R-032). |
| T5 | **Runaway cost or loops** | Token budgets per job and per day (R-1205); run limits (R-120); parking; idle alerts; max session length (R-1206). |
| T6 | **Malicious plugin or pack** | Preview of commands before install, pinning to a sha, abort on manifest change, scoped plugin tokens, no installs from phones, `XHERD_PLUGIN_INSTALL=off` (§10.5). No sandbox (stated). |
| T7 | **Remote access abuse** | Token on every request; loopback binding by default; LAN and remote access off by default; HTTPS for remote; device pairing with one-time codes; phones get steering rights only; Origin checks against CSRF; token rotation revokes all phones (R-1207). |
| T8 | **Untrusted content in the UI** (XSS via job-written Markdown, PR comments, diagrams) | Sanitised Markdown; Mermaid in strict mode without HTML labels; no remote images in job content by default; panel plugins in sandboxed iframes without the token. |
| T9 | **Secret leakage** in logs, hook output, PR text or specs | Redaction of known secrets in every captured output; secrets passed only on stdin; secret tripwire; secrets never in `instructions.md`. |
| T10 | **Supply chain** of the image and binary | Pinned versions of bundled CLIs, signed releases and images, SBOM (§14.4). |

### 13.3 Requirements

| ID | Requirement |
|---|---|
| R-1201 | Untrusted text handed to jobs (PR comments, ticket text fetched by xherd, hook output) is wrapped in delimited blocks labelled as untrusted data; xherd rules tell jobs never to follow instructions found inside them. |
| R-1202 | Tokens are scoped: user token (full), device tokens (full or phone rights), job tokens (`/api/v1/hooks`, `/api/v1/xap` for one job), plugin tokens (declared scopes). Job and plugin tokens are created at start and revoked when the session or plugin stops. |
| R-1203 | In the container, the daemon runs as its own user; `config.json`, `secrets.json` and the database are readable only by it. Jobs run as the host-mapped user and reach git credentials only through the credential helper, which asks the daemon over a socket and answers only for configured hosts. In native mode the same files are 0600, which is a guardrail, not a boundary (same OS user); the UI says so. (**verify** ownership behaviour on macOS bind mounts.) |
| R-1204 | Each worktree gets an xherd `pre-push` hook (chained with any existing one) that allows pushing only the job's branch to its configured remote, never deletes remote refs, and refuses non-fast-forward pushes. Hooks can be skipped (`git push --no-verify`), so this prevents mistakes, not a determined job; **branch protection on the host is the real boundary**, and `xherd security check` reports whether the base branch is protected. |
| R-1205 | Budgets: per-job token budget and a global daily budget (tokens; cost estimate where the runtime reports it). Reaching 80 % notifies; reaching 100 % pauses the job at its next stop. |
| R-1206 | `maxSessionMinutes` (default off): a session running longer is asked to hand off and is paused. |
| R-1207 | Network: the daemon listens on loopback by default; LAN (phones) and remote hostnames are opt-in; other connections are dropped before HTTP. |
| R-1208 | Audit: approvals (ledger), XAP calls, plugin installs/updates, settings changes and token rotations are recorded as events and kept for `eventRetentionDays` (default 90). |
| R-1209 | Unattended mode (`never` autonomy) requires a one-time confirmation that lists what will happen without the user (draft PRs, no ticket updates). |
| R-1210 | `xherd security check` summarises the current exposure: network bindings, autonomy levels in use, tool permissions `all`, plugins without a pinned sha, branch-protection hints, budgets not set. |

## 14. Updates and migrations

### 14.1 Versioning

| ID | Requirement |
|---|---|
| R-1301 | Semantic versioning for the binary and image. Compatibility promises cover the XAP major version, the plugin protocol, the flow-pack manifest (`ui:` and schema versions) and the HTTP API (`/api/v1`). A breaking change bumps the major, and the previous major stays supported for at least one minor release. |
| R-1302 | Image tags: `X.Y.Z`, `X.Y`, `X` and `latest`; the compose file pins the major (`:1`). Channels: `stable` and `edge`. |

### 14.2 Updating

| ID | Requirement |
|---|---|
| R-1303 | The UI shows when a newer version exists (a daily check of the release feed; can be disabled; no other data is sent). |
| R-1304 | **Draining**: restarting the container ends every tmux session, so `xherd docker update` (host) first asks live jobs to hand off and pauses them (or waits until none is running, with a timeout and a "force" option), takes a backup (§15), pulls the new image, recreates the container, and resumes the paused jobs (same runtime sessions where possible, else fresh sessions). Native updates replace the binary and restart only the daemon; jobs keep running in tmux. |
| R-1305 | Database migrations are forward-only, numbered, run at start inside a transaction, after an automatic copy of the database. A failed migration leaves the old database in place and the daemon refuses to start with the error. Downgrades restore from that copy. |
| R-1306 | Config files carry a schema version and are migrated the same way; unknown keys are kept. |
| R-1307 | Built-in prompts (xherd) are re-installed on update with the install-manifest rules (R-203): user-edited files are kept and the new version is written next to them with a warning (Setup checks, R-203). At each start the daemon also refreshes the hook file and pre-push guard of every unfinished job when this version writes something different (new binary path, events, allowed calls), keeping the user's own hook entries. |
| R-1308 | **In-flight jobs keep their pack version**: a job records the pack version it started with and uses it until done, unless the user upgrades it explicitly. |
| R-1309 | Plugins: updates are explicit (R-543); an update that requires a newer xherd is refused with the needed version. |
| R-1310 | Release notes list breaking changes, migrations and any manual step; the UI shows them after an update. |

### 14.3 Toolchains and agent CLIs

| ID | Requirement |
|---|---|
| R-1311 | Coding-agent CLIs bundled in the image are pinned per image version; Settings shows their versions, and a check warns when the installed CLI is outside the tested range of its runtime plugin. Users can update a CLI inside the container (kept in `xherd-home`) at their own risk. |
| R-1312 | Toolchains in `xherd-home` survive image updates; the image's version managers can be updated without reinstalling languages. |

### 14.4 Releases

| ID | Requirement |
|---|---|
| R-1313 | Release artefacts: static binaries (linux amd64/arm64, darwin amd64/arm64), multi-arch image, checksums, signatures (cosign or minisign), SBOM; Homebrew tap and install script. |
| R-1314 | Every release passes the full test suite, plugin and pack conformance for built-ins, the Docker smoke test, and a migration test from the previous minor. |

## 15. Backups and restore

### 15.1 What to back up

| Data | Where | Default | Why |
|---|---|---|---|
| Database, config, secrets, devices | `<data>` in `xherd-home` | ✓ (secrets encrypted) | Jobs, profiles, approvals, settings |
| Plugins and packs | `<data>/plugins/` | ✓ | Reinstalling may fetch different code |
| Saved agents, their versions and memories | database | ✓ (in the database copy) | Setups jobs start from (R-035) |
| Skills library | `<data>/skills/` | ✓ | Agents list skills by name |
| Archived `.xherd/` folders | `<data>/archive/` | ✓ | History of finished jobs |
| Live jobs' `.xherd/` folders | worktrees | ✓ | Specs, ledgers and handoffs are not in git |
| Unpushed work | worktrees | ✓ as git bundles of job branches + patches of uncommitted changes | Not on the remote yet |
| Runtime logins | `xherd-home` (`~/.claude`, `~/.codex`, `~/.config/gh`) | optional (encrypted) | Re-login is easy but annoying |
| Toolchains, package caches, clones | `xherd-home`, workspace | ✗ | Reproducible |

### 15.2 Requirements

| ID | Requirement |
|---|---|
| R-1401 | `xherd backup create [--to <dir>] [--include-logins]` writes one archive: a consistent database copy (SQLite online backup), config, plugins, archives, live `.xherd/` folders, git bundles of job branches with commits not on the remote, and patches of uncommitted changes. Secrets and logins are encrypted with a passphrase or excluded. |
| R-1402 | Scheduled backups (Settings → Backups): daily when a destination is set, keep N (default 7), to a host-mounted folder; an optional `command` destination (e.g. `rclone copy`, `restic backup`) runs after each backup. |
| R-1403 | `xherd backup restore <archive>` (daemon stopped): restores data and plugins, re-creates `.xherd/` folders and job branches from bundles in existing worktrees (or lists what could not be restored), and marks restored live jobs `paused` so the user resumes them deliberately. |
| R-1404 | `xherd backup verify <archive>` checks integrity (hashes, database check, bundle verification) without restoring. |
| R-1405 | Updates (R-1304) and migrations (R-1305) always take a backup first; the last 3 of these are kept separately. |
| R-1406 | Kubernetes: the Helm chart documents PVC snapshots for the home and workspace volumes in addition to `xherd backup`. |
| R-1407 | Export/import of profiles and settings as JSON (without secrets) for moving to another machine. |

## 16. Supporting features

### 16.1 Repo profiles

Matched by remote URL, then path. Fields: name, colour, match; commands (run, test, build, lint, extras; each runs in its own tmux window from the job page and can be stopped); extra instructions; untracked files (copy/symlink); base/staging branches; default flow and pack; ticket prefix; autonomy, sensitive paths, max diff lines; tool permissions; runtime/model/driver per step; plugin setting overrides; stage hooks; toolchain and setup; spec folder mode; budgets. Export/import as JSON.

### 16.2 TODOs

Reminders (due at a time, once or repeating) and **condition checks**: at each due time xherd starts a short read-only session of a chosen runtime and model, with a system prompt that forbids changes and asks for a final line `XHERD_RESULT {"met": bool, "summary": "…"}`. Met → needs you; not met → rescheduled; a question or permission prompt, or 2 minutes of silence → needs you (terminal shown); failure → error. Actions: complete, reschedule, check now, edit, delete.

### 16.3 Notifications

Browser notifications (permission asked on the first click), the desktop app later, OS notifications from a native daemon when no browser tab is open and a desktop exists, and **notifier plugins** (ntfy built in; Telegram and Slack webhooks as plugins). Triggers: gate opened, auto-approved, blocked, delivery failed, idle alert, new PR comments, CI failed / green again, conflicts, budget thresholds, TODOs. One notification per job and kind replaces the previous one.

### 16.4 Work log and tokens

Job time (time in `running`) and the user's time on a job page, per day and ticket. Token usage per runtime session from the runtime's `usage` command or the stop payload; totals per job; cost estimate where known.

**Delivery metrics per ticket.** Jobs can feel faster than they measure, so the Work log also records, per ticket: cycle time (job created → PR merged or closed), the review→fix rounds the job went through (the R-120 counters), and how many auto-approvals were later followed by a change request on the same gate (rework). The measured result, not the feeling, is what the UI shows.

### 16.5 `instructions.md`

Rewritten on every start, resume and autonomy change: workspace notes (folder jobs), autonomy level and what it means for the job, ticket key and link with the tracker's fragment, git host fragment, repo commands (or a note to use the project's usual ones and state them in evidence), repo instructions from the profile, the XAP commands the coding agent must call, its port block, and the untrusted-content rule.

### 16.6 Memory proposals

Memory has three levels: `memory.md` (one job, carried between its steps), the saved agent's memory (every job of that agent, R-039) and the repository's `AGENTS.md` (every job there). Each is changed by a proposal the user accepts.

`memory.md` is one job's memory and goes with it; the repository's `AGENTS.md` is the memory shared by every future job there (#65). The review step, once all tasks are done, may propose an update of `AGENTS.md` (the whole proposed text plus a one-line reason; a starter `AGENTS.md` when none exists): short facts a future job needs (commands, conventions, traps), no task history. `CLAUDE.md` only imports it (`@AGENTS.md`), so every runtime reads the same text. The job page's Shared memory tab shows the diff (and the `CLAUDE.md` change, if any); the user accepts, optionally editing first, and xherd commits `AGENTS.md` (and `CLAUDE.md` when the import is missing) on the job's branch as one commit, so it ships with the PR; or dismisses it (dismissing a starter file is remembered per repo, and jobs stop proposing one there). Accepting is refused only while the job works. Once its PR is open, the accept still commits on the branch; xherd never pushes, so the user pushes it to update the PR. Pending proposals do not block the PR.

## 17. User interface

### 17.1 Principles

- One colour means **"needs you"** and is used for nothing else; actions, running, errors and done each have their own tone.
- Dense, calm layout for many jobs at once; monospace for names, branches, commits and paths.
- View state lives in the URL (bookmarkable; Back works).
- In-app dialogs for errors (long, copyable messages) and confirmations; never browser `alert`/`confirm`.
- Job-written and host content is untrusted (§13 T8).
- Light and dark themes; reduced motion respected; visible focus; reachable by keyboard.
- Everything live over one websocket; a reconnect reloads the full state.
- English UI in v1, strings kept in one catalogue for later translation.

### 17.2 Screens

| Screen | Content |
|---|---|
| **Overview** | "Needs you" first (longest wait first: what is waiting and the one useful action), then every job: status, stage, task progress, branch, tokens, last activity; filters. |
| **Job page** | Header (name, ticket link, status, autonomy, runtime, tokens, actions); gate box (what to approve, held reason, assumptions, the job's reply, Approve / Request changes, review changes for commits); message box; rail of panels from the view declaration (terminal or transcript, documents with "changed since reviewed", tasks, commits, diff with "viewed" marks, PR, issues, memory, log); profile commands; Open in. |
| **Inbox** (was Needs attention) | Jobs waiting for you, due TODOs, and auto-approved items with reason, assumptions, diff since last review, Mark seen. |
| **Kanban** | Tasks of all jobs plus stage cards and TODOs; filters and swimlanes in the URL. |
| **New job / New TODO** | Generated form including pack inputs; ticket link preview; repo picker or clone; branch list. |
| **Settings** | General, Notifications, Open in, Git access, Toolchains, Repo profiles, Plugins (install, settings, ACP badges), Runtime per step, Backups, Remote access. |
| **Setup checks / Welcome** | First-run wizard: where it runs → plugins (runtime, tracker, host, toolchain) → commit identity → git hosts → checks → ready. Toolchains are plugins, so they are set up with the others (installs only in the container, R-653). Until required checks pass, other screens are disabled. |
| **Work log** | Job time and user time per day and ticket; per-ticket cycle time, review–fix rounds and auto-approval rework (§16.4). |

### 17.3 Phones

Phones (LAN with a token link or QR code, optional HTTPS with a local CA) get **steering rights**: view everything except settings and terminals; approve, request changes, answer, decide PR comments, approve replies, pause, resume, add notes, handle TODOs, answer ACP permission prompts. No terminals, commands, settings, plugin installs or new jobs.

## 18. CLI

```
xherd [start] [--no-open]               start the native daemon if needed and open the UI
xherd daemon                            run the daemon in the foreground
xherd stop | restart | status
xherd service install|uninstall         systemd user unit / launchd job
xherd docker init|doctor|update         host: compose setup, checks, drained update
xherd login [URL]                       pair this CLI with a daemon (host companion)
xherd new <NAME> [--ticket URL|KEY] [--flow f] [--pack p] [--repo dir|url] [--base b]
          [--runtime r] [--model m] [--autonomy a] [--mode p] [--driver terminal|acp] ["prompt"]
xherd ls | attach | open | shell | pause | resume | approve | cancel <job>
xherd changes <job> "comment"         request changes at the open gate
xherd talk <job> "text" | finish <job>   an ask session on a stopped job, and its end
xherd resolve-conflicts <job>         merge the PR's base branch into the job's branch
xherd remove-worktree <job>           delete a finished job's worktree (cancel --remove-worktree too)
xherd mode | model | autonomy <job> … per-job tool permissions, model, autonomy
xherd pause --all                       kill switch
xherd pair                              one-time pairing link
xherd install [--runtime r | --all] [--pack p]
xherd flow ls|install|remove|test|new
xherd sdd check|try <dir>
xherd plugin ls|new|test|link|unlink|search|install|update|remove
xherd git doctor [host] [--repo URL]
xherd backup create|restore|verify
xherd security check
xherd hook <event> | report …           status reporting (silent)
xherd gate|stage|tasks|task|commit|pr|handoff|ask|issue|memory …   XAP (xap.md)
xherd mcp                               XAP as an MCP stdio server
xherd credential                        git credential helper
xherd terminal-handler install|uninstall   register the xherd-term:// link (Open in terminal)
```

`<job>` is a name or a unique ticket key.

## 19. Non-functional requirements

| ID | Requirement |
|---|---|
| R-901 | `xherd hook` and `xherd report` finish in < 10 ms when the daemon is up, never print to stdout, never exit non-zero. |
| R-902 | 20 live jobs and 200 total stay responsive (API p95 < 100 ms on a laptop). |
| R-903 | Single static binary with the UI embedded; image size kept small (target < 400 MB without languages, agent CLIs included, **verify**). |
| R-904 | All state survives daemon restarts; jobs survive daemon restarts (not container restarts, see R-1304). |
| R-905 | Structured JSON logs; optional OpenTelemetry export (traces around reports, delivery, plugin commands). |
| R-906 | No telemetry. The only outbound call made by xherd itself is the optional update check. |
| R-907 | Supported platforms: Linux and macOS (native and Docker); Windows through WSL2 or Docker. |

## 20. Beyond coding (vision)

The core (sessions, stages, gates, autonomy, per-step models, XAP, ledgers, views) is not specific to code. Three seams keep other work possible: **workspaces** (`git-worktree`, `folder`, later read-only PR checkouts), **artifacts** (any file type, approvable by hash, shown by viewer panels) and **delivery** (PR today; docs or media publishing later).

| Flow | Effort on top of v1 |
|---|---|
| Review someone else's PR (read-only checkout, draft comments ✋, post) | Low |
| Research and write a design (folder, web/MCP tools, outline ✋, draft ✋, publish) | Low–medium |
| Prepare and create a video (script ✋, storyboard ✋, rough cut ✋, render, upload) | High: media viewers, long-running work, large files, paid APIs |

| ID | Requirement |
|---|---|
| R-1001 | Workspace kinds behind one interface; git features shown only when the workspace supports them. |
| R-1002 | The job page is built from workspace and pack capabilities (§11). |
| R-1003 | Packs declare artifacts per stage; gates may point at any artifact. |
| R-1004 | Ticket keys are optional for non-coding flows. |
| R-1005 | Later: `delivery` plugins, tracked **background work** for long renders, budgets covering paid tool APIs. |

## 21. Roadmap

1. **v1.0**: MVP scope (§2.3).
2. **v1.x**: Codex and opencode runtimes; Kanban, To review, Work log; Welcome toolchain installer and toolchain plugins; plugin install from the UI and catalogue; TODOs; `never` autonomy.
3. **ACP driver** per runtime, selectable per job.
4. Server and Kubernetes deployment; VS Code Remote-SSH entries.
5. **Electron desktop app** with tray, notifications and built-in host companion.
6. Panel plugins; declarative views for external packs; PR-review flow pack; research/design flow pack.
7. Per-job containers with egress allowlists; parallel task execution inside one job (sub-worktrees, different models per task).
8. Multi-repo tickets.

## 22. Decisions

| # | Decision |
|---|---|
| D-1 | Daemon in **Rust**, one static binary with the web UI embedded. |
| D-2 | **Docker-first** with same-path workspace mounts; native binary supported; server/Kubernetes with reduced Open-in. |
| D-3 | **One shared container** with all tools in v1; version managers for languages. |
| D-4 | **Single user** per daemon. |
| D-5 | SDD documents **private** in `.xherd/` by default; committing is opt-in per profile. |
| D-6 | Approvals by **content hash** in an append-only ledger; the daemon never edits job documents except the progress lines (task and commit status, sha, evidence) it owns; hashes ignore them. |
| D-7 | Pluggable SDD: flow packs, step roles, and dynamic flows via XAP. |
| D-8 | Driver chosen by configuration: live terminal (default) or ACP. |
| D-9 | Runtimes declare models with tiers and effort; steps choose runtime + model/tier + effort + driver. |
| D-10 | Plugin kinds include `toolchain` and `panel`; the CLI is the plugin API; plugins installable from the UI, pinned and previewed; `xherd plugin new` scaffolds them. |
| D-11 | **Declarative views** with a fixed vocabulary; no visual workflow builder. |
| D-12 | The repo's PR template takes priority. |
| D-13 | Git over HTTPS and SSH, per host, with guided setup and a doctor. |
| D-14 | Desktop app in **Electron**, post-v1. |
| D-15 | Default port **4740**. |
| D-16 | **GPL-3.0-or-later** licence (was MIT, #134). |
| D-17 | Not pursued: jobs controlling other jobs, a combined view of several daemons, replacing tmux with another terminal server. |
