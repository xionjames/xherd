# xherd — Product Specification

> Status: **draft v1.0** · 2026-11-06
> Companions: [`design.md`](./design.md) (architecture and implementation), [`xap.md`](./xap.md) (agent protocol reference).
> Requirement IDs (`R-xxx`) are stable and referenced from the other documents. Claims not yet verified against the tools they depend on are marked **(verify)**.

---

## Contents

1. Summary
2. Goals, non-goals and MVP scope
3. Vocabulary
4. Scenarios
5. Agents
6. Gates, approvals and autonomy
7. xhSpec: the built-in SDD
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

You give xherd a ticket and a repository. It creates an isolated git worktree and a terminal session, starts a coding-agent CLI (Claude Code, Codex, opencode…) with a **spec-driven development (SDD) flow**, stops the agent at **approval gates**, lets you approve or ask for changes from a web UI (browser, phone, later a desktop app), and follows the resulting pull request until it is merged.

Key properties:

- **Many agents in parallel**, each in its own worktree and branch, with a concurrency cap and a queue.
- **Agents outlive the daemon**: they run in tmux; the daemon can restart without stopping them.
- **The SDD is pluggable**: xhSpec ships built in; other SDDs plug in as flow packs or by calling the xherd agent protocol (XAP) from their own commands.
- **You stay in control** through gates, autonomy levels with tripwires, and an audit trail of what was approved.
- **Docker-first**, single Rust binary, MIT licensed.

## 2. Goals, non-goals and MVP scope

### 2.1 Goals

- G1. Run several coding agents on separate tickets at once, each isolated in its own worktree and branch.
- G2. Keep a human in the loop at the points that matter (spec, design, commits, PR), with adjustable autonomy.
- G3. Make the SDD replaceable: xhSpec built in, other methodologies through packs or the XAP contract.
- G4. One binary, no runtime dependencies apart from `git`, `tmux` and the agent CLIs; the status hook starts in milliseconds.
- G5. Docker is the easiest way to run xherd, without losing "open this worktree in my IDE".
- G6. Runtime-agnostic: Claude Code, Codex CLI, opencode and others through runtime plugins; live terminal or ACP driver per configuration.
- G7. Coding is the first-class use case, but the core stays domain-neutral where it costs little (workspaces, artifacts, delivery and views are pluggable), so other kinds of work can be added later (§20).

### 2.2 Non-goals

- N1. xherd does not call tracker APIs from its core. Agents read tickets through MCP or CLIs; tracker plugins may be called only from stage hooks, opt-in.
- N2. No multi-tenant service. **One daemon serves one person**; teams share work through PRs and tickets.
- N3. The daemon makes no LLM calls itself. All model work happens inside agent runtimes.
- N4. No native Windows build (WSL2 or Docker only).
- N5. No per-agent containers in v1: all agents share one container with all tools.
- N6. **No visual workflow builder.** Flows and views are declared in text (YAML) inside packs and plugins.

### 2.3 MVP scope (v1.0)

The full spec is larger than one release. v1.0 ships a usable tool; the rest follows in minor releases.

| Area | v1.0 | Later |
|---|---|---|
| Runtimes | Claude Code (terminal driver) | Codex, opencode, ACP driver |
| SDD | xhSpec built in; dynamic flows (§8.6) | External packs with `parse`, panel plugins |
| Hosts / trackers | GitHub; tracker `none` and Jira (link only) | GitLab, Linear… |
| Deployment | Local Docker, native binary (Linux, macOS) | Server/Kubernetes, Electron app |
| UI | Overview, Agent page, Settings, Setup checks, Welcome | Kanban, To review page, Work log, command palette |
| Autonomy | `ask` and `important` with tripwires | `never` (unattended) after the guardrails have run in practice |
| Plugins | Built-in plugins, `xherd plugin new/test/link` | Install from the UI, catalogue |
| Toolchains | Version managers in the image; per-worktree resolution | Welcome installer, toolchain plugins |
| Other | Guardrails §13, backups §15, updates §14 | TODOs, budgets, notifications beyond browser + ntfy |

## 3. Vocabulary

| Term | Meaning |
|---|---|
| **Daemon** | The long-running xherd service: HTTP API, websockets, web UI, timers. Default port **4740**. |
| **Agent** | One coding-agent run for one ticket: workspace (worktree + branch) + terminal session + runtime session(s). Named `<KEY>` or `<KEY>-<slug>`. |
| **Runtime** | A coding-agent CLI described by a runtime plugin (`claude-code`, `codex`, `opencode`). |
| **Driver** | How xherd talks to a runtime: `terminal` (CLI in tmux, live terminal; default) or `acp` (Agent Client Protocol). |
| **ACP** | Agent Client Protocol: open JSON-RPC-over-stdio protocol between a client and a coding agent (prompts, streamed messages, tool calls, plans, permission requests). |
| **Tracker / Host** | Plugins for the ticket system (link format, key pattern, prompt fragment) and the git host (PR commands, templates). |
| **SDD** | Spec-driven development: a method that moves work through written documents and approvals before and while coding. |
| **xhSpec** | xherd's built-in SDD. Commands `/xhspec:*`. |
| **Flow pack** | A versioned bundle that defines an SDD for xherd: manifest, prompts per runtime, optional schemas, parser and views. |
| **Flow** | A path through a pack's stages (`feature`, `bug`, `hotfix`), or `none` (no SDD). |
| **Stage / step role** | A pack's step (its own id) and the canonical role it maps to: `explore`, `spec`, `design`, `tasks`, `implement`, `review`, `ship`, `pr`, `custom`. |
| **Gate (✋)** | A point where the agent stops and waits for approval: documents, a proposed commit, PR triage, PR replies, or a question. |
| **Autonomy** | `ask` / `important` / `never`: which gates xherd may approve by itself. |
| **Tripwire** | A condition that makes a gate wait for a human despite autonomy. |
| **Fresh session** | After an approved gate, the runtime session is replaced by a new one started with the next command (saves context and tokens). |
| **Parking** | Ending the session of an agent that waits at a gate too long, to free a slot. |
| **Watching** | The agent's session has ended; xherd polls its open PR. |
| **XAP** | xherd agent protocol: `xherd` subcommands an agent or SDD calls to declare gates, tasks, commits, PRs and status ([`xap.md`](./xap.md)). |
| **Workspace** | Where an agent works: a git worktree (default) or a plain folder. In Docker mode the workspace root is mounted at the **same absolute path** on the host and in the container. |
| **Private folder** | `.xherd/` in the worktree, excluded from git: agent metadata, instructions, SDD documents (by default), ledgers. |
| **Ledger** | Append-only JSONL files: approvals (with document hashes) and discovered issues. |
| **Repo profile** | Per-repository settings matched by remote URL, then by path. |
| **Host companion** | The `xherd` binary on the host when the daemon runs in Docker: attach terminals, open IDEs. |
| **Tier** | Abstract model level (`high`, `medium`, `low`) mapped by each runtime to a concrete model. |

## 4. Scenarios

- **S1. Laptop.** `docker compose up -d`, open `http://127.0.0.1:4740`, start three tickets, approve specs from the phone, open a worktree in IntelliJ with one click.
- **S2. Home server.** Daemon on a server behind HTTPS, browser paired as a device, VS Code in the browser or Remote-SSH for edits.
- **S3. Team SDD.** A team adds the XAP calls to its own SDD commands; xherd shows its gates and tasks without a pack (§8.6).
- **S4. Mixed SDDs.** Ticket SC-1 runs with xhSpec, SC-2 with another SDD, in the same repo; neither leaves files in the repo; the "why" is in each PR description.
- **S5. Mostly unattended.** Autonomy `important`, a stage hook runs e2e tests before ship, a draft PR appears; the user reviews the auto-approved steps afterwards.

## 5. Agents

### 5.1 Creating an agent

| ID | Requirement |
|---|---|
| R-001 | Input: ticket key, optional slug, repository (path or URL), optional base branch, flow, prompt, autonomy, tool permissions, runtime/model per step, driver, workspace kind. |
| R-002 | Everything is validated **before** anything is created: key format (`^[A-Z0-9][A-Z0-9_-]*$`, upper-cased) and the tracker's narrower pattern; slug `^[a-z0-9][a-z0-9-]{0,40}$`; the repo is a git repo (fetched first if xherd cloned it); profile resolved; plugins usable; flow exists in the selected pack (`none` requires a prompt); requested tool-permission mode exists in every runtime used, and `all` requires the sandbox flag; agent name, branch and worktree path are free; base branch resolves (profile → repo default; a plain name in a clone resolves to `origin/<name>`). Any failure returns one clear error and changes nothing. |
| R-003 | A valid agent is saved as `queued` and started when a slot is free (§5.3). |

### 5.2 Launching

| ID | Requirement |
|---|---|
| R-004 | Create the worktree with `git worktree add --no-track -b <name> <path> <base>` (no upstream, so a plain push cannot land on the base branch). |
| R-005a | Copy or symlink the profile's untracked files (e.g. `.env`) into the worktree. |
| R-005b | Install the runtime's status reporting (e.g. Claude Code hook entries in `.claude/settings.local.json`, tagged so they can be replaced; refuse if the existing JSON is invalid) and add every xherd-written path to `.git/info/exclude`. |
| R-005c | Write `.xherd/agent.json` (`{id, ticketKey, name, pack, flow}`) and `.xherd/instructions.md` (§16.5). |
| R-005d | Resolve the toolchain for the worktree (§12.4) in its own tmux window, then start the runtime in a tmux session named after the agent, with `XHERD_ENV=1`, `XHERD_BIN`, `XHERD_AGENT_ID`, `XHERD_AGENT_TOKEN`, `XHERD_PORT_BASE` in its environment, and the flow's start command as the first prompt. |

### 5.3 Lifecycle

Statuses: `queued`, `running`, `waiting-input`, `blocked`, `idle`, `parked`, `paused`, `watching`, `done`, `cancelled`, `error`.

| From | To | When |
|---|---|---|
| queued | running | A slot is free (live agents < `maxConcurrentAgents`, default 4) |
| running | idle | The agent stopped and nothing waits on the user |
| running | waiting-input | The agent stopped at a gate or asked a question |
| running | blocked | The terminal shows a dialog (permission prompt) or the agent reported `blocked` |
| blocked | running | Work resumed / dialog gone |
| idle, waiting-input | running | User message, approval or change request |
| waiting-input | parked | Waiting longer than `gateParkMinutes` (default 30; 0 = never) |
| parked | queued | Approve or message (needs a slot again) |
| running | paused | User paused (handoff written, session ended) |
| paused, cancelled, error | queued | Resume |
| idle | watching | PR open and nothing left to do |
| watching | queued | New PR review comments |
| watching | done | PR merged or closed |
| running | error | The terminal session disappeared |
| any live | cancelled | User cancelled |

| ID | Requirement |
|---|---|
| R-006 | **Live** statuses (`running`, `waiting-input`, `blocked`, `idle`) count against the cap. TODO checks do not. |
| R-007 | **Needs you** = `waiting-input`, `blocked`, or an open gate. |
| R-008 | Reports from a session xherd already ended (paused, queued, cancelled) never revive the agent. |
| R-009 | Every status report carries a per-agent **monotonic sequence number**; reports with a lower or equal number are logged but do not change state. Reports synthesised by the daemon (screen rules) get daemon sequence numbers and never make a real report stale. |
| R-010 | Every change to an agent goes through one serialised path per agent; no change is lost when reports, user actions and timers arrive together. |
| R-011 | **Reconcile** on start and every minute: a live agent whose tmux session is gone → `error` (event `session-lost`). |

### 5.4 Status reporting

| ID | Requirement |
|---|---|
| R-012 | Runtimes with hooks call `xherd hook <event>` (hook JSON on stdin). The command must never print to stdout or exit non-zero, because runtimes may show hook output to the model. |
| R-013 | Any agent or wrapper can push status with `xherd report --state working|idle|blocked [--message] [--session] [--resume-cmd] --seq N`, and `xherd report --release` on exit. This is the contract for third-party agents ("add xherd support to your agent"). |
| R-014 | The CLI finds its agent through `XHERD_AGENT_ID` or `.xherd/agent.json` in the current directory or a parent, and posts to the daemon. If the daemon is down, it appends to `hook-queue.jsonl`, replayed on daemon start in order and de-duplicated by event id. |
| R-015 | Event effects: session start → `running`; prompt submitted → `running`, acknowledges delivery, records "user prompted" when it is not a slash command; tool use / `working` → `running` (not logged; frequent); dialog notification → `blocked` + notify; stop / `idle` → `idle` or `waiting-input` (gate open or the last message asks a question), records running background tasks, triggers token usage, auto-approval, PR lookup, parking. |
| R-016 | Each report also updates the runtime session id, token usage, and last activity time, and interrupts a running auto-approval check when new work starts. |
| R-017 | **Reply detection**: at a gate, if the agent's last message asks something, or it stopped without changing the gate documents after the user's message, the UI shows the agent's reply and asks for the user's answer. |
| R-018 | **Screen rules** for runtimes whose dialogs have no hook: every 3 s, the last N lines of the pane are matched against the runtime's patterns; a match reports `blocked`, its disappearance `working`. Real reports win. |
| R-019 | **Idle alert**: an agent idle longer than `idleAlertMinutes` (default 15) notifies once. |

### 5.5 Sending text to agents (delivery)

| ID | Requirement |
|---|---|
| R-020 | Terminal driver: text is typed into the agent's window, then Enter (runtimes may need extra Enters after slash commands). ACP driver: an ACP prompt message. |
| R-021 | xherd never types into a `blocked` agent. |
| R-022 | Delivery is acknowledged by the next prompt-submitted or `working` report within a timeout (default 10 s); otherwise Enter is retried once and the user is notified. |
| R-023 | **Hand-on** after an approved gate: with fresh sessions on, only the agent window's process is replaced (`tmux respawn-window`) by a new runtime session started with the next command, so other windows (a dev server, tests) keep running; otherwise the command is typed into the current session; a parked or watching agent is started with it. |
| R-024 | No fresh session while the agent has background tasks running (they would be killed), or for flow `none`. |
| R-025 | A change of runtime between steps always uses a fresh session in the new runtime (status files and permissions for that runtime are written first). If the new runtime is unusable, the agent stays and the user is warned. |

### 5.6 Pause, resume, park, cancel, remove

| ID | Requirement |
|---|---|
| R-026 | **Pause**: ask the agent to write a handoff (pack `handoff` command), wait for it to stop (≤ `pauseTimeoutSec`, default 120), end the session, status `paused`. Flow `none`: end the session. |
| R-027 | **Resume**: back into the same runtime session when the runtime can resume it, otherwise (or when a handoff was written and fresh sessions are on) a fresh session with the pack's `resume` command. A pending "next prompt" set by xherd (e.g. PR review) takes precedence. |
| R-028 | **Park** (automatic, §5.3): end the session; the agent stays in Needs you. A message resumes the same session; approval starts a fresh one for the next step. |
| R-029 | **Cancel**: end the session without handoff; keep the worktree; resumable. |
| R-030 | **Remove worktree** (done or cancelled agents only): archive `.xherd/` to `<data>/archive/<repo>/<agent>/`, `git worktree remove`, keep the branch. |
| R-031 | **Delete**: remove the agent's records (not the branch). |
| R-032 | **Pause all** (UI and `xherd pause --all`): a kill switch that pauses every live agent at once (§13). |

### 5.7 No-workflow and folder agents

| ID | Requirement |
|---|---|
| R-033 | Flow `none`: the prompt is sent as is; no gates, tasks or handoff. Status, pause/resume, PR follow-up (notify only) still work. |
| R-034 | Workspace `folder`: the agent runs in `<data>/scratch/<name>/` (or a chosen folder) without a branch; git-dependent features are hidden. Instructions tell it which repo it may read. |

## 6. Gates, approvals and autonomy

### 6.1 Gates

| ID | Requirement |
|---|---|
| R-101 | Gate kinds: `docs` (one or more documents of a stage), `commit` (one proposed commit), `pr-triage` (new PR comments analysed), `pr-replies` (drafted replies), `question`. |
| R-102 | Gates are **declared** by the agent through XAP (`xherd gate open …`, `xherd commit propose …`, `xherd ask …`) and validated against the files. If an agent stops with an unapproved gate document but declared nothing, xherd infers the gate from the files and logs `gate-inferred`. |
| R-103 | Priority when several apply: commit > PR triage/replies > documents > question. |
| R-104 | **Approve**: append an approval to the ledger (R-106), snapshot the reviewed text for "changed since you reviewed", log the event, and hand on the next command (§5.5). |
| R-105 | **Request changes**: send the agent a message naming the gate, the files and the user's comment, asking it to revise and stop at the gate again. |

### 6.2 Approval by content hash

| ID | Requirement |
|---|---|
| R-106 | Every approval (human or automatic) appends `{gate, file, sha256, actor, auto, reason, at, commit}` to the agent's approval ledger and the daemon database. The daemon **never edits** the agent's documents. |
| R-107 | A document is approved when the ledger has an approval whose hash equals its current content (line endings normalised). Editing an approved document re-opens its gate ("changed since approved") and the UI shows the diff against the approved version. |
| R-108 | Several documents changed together form **one** gate approved together. |

### 6.3 Autonomy

Each stage has an **autonomy class**. The level decides which classes xherd may approve:

| Class | `ask` | `important` | `never` | xhSpec stages |
|---|---|---|---|---|
| `critical` | wait | wait | approve | spec, ship |
| `routine` | wait | approve | approve | design, tasks, review |
| `code` | wait | approve | approve | commit |
| `human-only` | wait | wait | wait | PR triage, PR replies, hotfix ship |

| ID | Requirement |
|---|---|
| R-109 | Autonomy is set per agent (default from profile, then Settings) and can change live; `instructions.md` is rewritten when it does. |
| R-110 | Auto-approval runs when the agent stops at an allowed gate: mark "checking", evaluate tripwires (some run commands), and approve only if the gate is unchanged and the agent did not ask anything. A user message in the meantime cancels the check; it re-runs at the next stop. |
| R-111 | Each auto-approval is listed in **To review** with the reason, the files, and the agent's stated assumptions, and notifies the user. |

### 6.4 Tripwires

A gate waits for a human, with the reason shown, when any of these hold:

| ID | Tripwire |
|---|---|
| R-112 | A document or commit is marked `needs-human: yes` (with a reason). |
| R-113 | The spec has open questions (when approving the spec). |
| R-114 | Review or ship: the review has blocker/major findings, or review-sourced tasks are not done. |
| R-115 | Code gates: changed files match the profile's **sensitive paths** (globs); the diff exceeds `maxDiffLines` (default 1000) or adds a file too large to count; the profile's **test command fails**. |
| R-116 | Commit gate: staged files differ from the task's declared `files:` (**scope fence**), except the agent's own spec folder in committed mode. |
| R-117 | Commit gate: the diff contains something that looks like a **secret** (built-in patterns for common token formats, private keys, `.env` content). |
| R-118 | A task's `verify:` command fails. |
| R-119 | A blocking **stage hook** failed (§8.5). |
| R-120 | **Run limits** reached: more than `maxAutoApprovalsInRow` (default 10) consecutive auto-approvals, more than `maxReviewLoops` (default 3) review→fix rounds, or the agent's token **budget** is used up. |
| R-121 | The flow is `hotfix` and the stage is ship. |

## 7. xhSpec: the built-in SDD

xhSpec is xherd's own SDD, written for xherd: documents for each step, approval gates, one commit per task, review in a fresh context, and PR rounds. Its prompts and formats ship as the built-in pack `xhspec`; their exact text lives in `packs/xhspec/` with `FORMAT.md` and JSON schemas (to be written).

### 7.1 Flows

- **feature**: spec ✋ → design ✋ (skippable) → tasks ✋ → implement (commit ✋ per task) → review ✋ → ship ✋ → PR rounds → done.
- **bug**: spec (investigation, root cause, fix approach) ✋ → tasks (a failing reproduction test first) ✋ → implement → review ✋ → ship ✋ → PR rounds → done.
- **hotfix**: like bug, based on the profile's `hotfixBase`; ship always waits for a human.
- **PR rounds**: new comments are analysed into items (triage ✋, a decision per comment), fixes are implemented with commit gates, replies are drafted (replies ✋) and posted.

Design may be skipped only when the change is small and local, has no data/API/config/dependency changes, touches nothing security- or concurrency-sensitive, and has one obvious approach. The spec states the decision; approving the spec approves the skip.

### 7.2 Commands

| Command | Does | XAP calls |
|---|---|---|
| `/xhspec:feature <KEY> <prompt>` | Read the ticket, explore, write `ticket.md` and `spec.md` | `gate open --role spec` |
| `/xhspec:bug <KEY> <prompt>` | Investigate without changing production code, write the bug `spec.md` | `gate open --role spec` |
| `/xhspec:design` | Write `design.md` | `gate open --role design` |
| `/xhspec:tasks` | Write `tasks.md` | `tasks sync`, `gate open --role tasks` |
| `/xhspec:next` | Implement the next ready task, verify, propose a commit; when all are done, start the review | `task start/done`, `commit propose` |
| `/xhspec:commit <Cnn>` | Commit exactly the proposed files, continue | `commit done` |
| `/xhspec:review` | Review the branch in a fresh context; findings become tasks; propose memory updates | `gate open --role review` |
| `/xhspec:ship` | Draft `pr.md` following the repo's PR template; after approval push, open a **draft** PR, apply the approved ticket update | `gate open --role ship`, `pr opened` |
| `/xhspec:pr-review` / `pr-apply` / `pr-fetch` / `pr-post` | PR rounds (fetch/post only for hosts without those plugin commands) | `gate open pr-triage / pr-replies` |
| `/xhspec:handoff` / `resume` / `status` / `rules` | Handoff note; reload and continue; print state; shared rules | `handoff written` |

### 7.3 Documents

Documents live in the agent's spec folder (§7.5): `ticket.md`, `spec.md`, `design.md`, `tasks.md`, `commits.md`, `review.md`, `pr.md`, `memory.md` (short state for fresh sessions), `pr-review.md`, plus private `HANDOFF.md` and `pr-comments.json`.

- Frontmatter carries `ticket`, `flow`, `stage`, optional `needs-human` + `reason`, `design: skip` (spec), `template` and `url` (pr). There is **no `approved` key**: approval lives in the ledger (§6.2).
- Gate documents end with the agent's **assumptions**; the spec also lists **open questions** (none → "None.").
- Tasks have stable ids (`T01`…), state (todo, doing, blocked, done), refs to requirements, `files:` (binding, R-116), `depends:`, `evidence:`, optional `verify:` command, optional model **tier** hint, optional **codebase contract** (verified file:line references and things that do not exist), and `source:` (spec, review, pr).
- Commits have ids (`C01`…), state (proposed, done, dropped), tasks, files, sha; one proposal at a time.

### 7.4 Additions

| ID | Requirement |
|---|---|
| R-131 | Discovered issues: `xherd issue add` records out-of-scope findings in the issue ledger; the UI lists them; at ship they can become follow-up tickets or TODOs. |
| R-132 | Second opinion: the review step can run on a different runtime/model than implement (ordinary per-step configuration, §10.3). Settings suggests it when one runtime is used everywhere. |
| R-133 | Task waves: tasks whose dependencies are all done form a wave, shown in the UI (parallel execution is roadmap). |
| R-134 | Taxonomy: optional `projects:` and `tags:` in spec frontmatter, used as filters and PR labels. |
| R-135 | Branch roles per profile: `baseBranch`, `stagingBranch`, `hotfixBase`. |

### 7.5 Where documents live

**Default: private.** A repo may host tasks driven by different SDDs, so nothing an SDD writes is committed by default:

```
<worktree>/.xherd/                  # in .git/info/exclude
  agent.json  instructions.md
  spec/       ticket.md spec.md design.md tasks.md commits.md review.md pr.md memory.md pr-review.md
  ledger/     approvals.jsonl issues.jsonl
  HANDOFF.md  pr-comments.json
```

| ID | Requirement |
|---|---|
| R-151 | By default SDD documents and ledgers live in `.xherd/` and are never committed. One agent uses one pack, recorded in `agent.json`. |
| R-152 | **The "why" travels in the PR**: at ship, the PR description carries the essentials (summary, requirements covered, key decisions, assumptions). |
| R-153 | **The repo's PR template comes first.** The host plugin's `pr-template` command returns the repo's templates (GitHub `.github/pull_request_template.md`, `.github/PULL_REQUEST_TEMPLATE/*.md`, `docs/`, root; GitLab `.gitlab/merge_request_templates/*.md`; organisation defaults via the API, **verify**). `pr.md` keeps every heading of the template, places the essentials in fitting sections, ticks checklist items only when true, and puts anything left over in a short `## Spec summary` at the end. With several templates the agent picks one and names it in frontmatter. Only without a template does xhSpec use its own layout. Optional per profile: a collapsed full copy of spec and design. |
| R-154 | Archived `.xherd/` folders stay viewable in the UI after the worktree is removed; optional pruning after N days (off by default). |
| R-155 | **Opt-in committed mode** per profile (`specDir: xhspec/{agent}`): documents and ledgers are written there and committed with the agent's normal commits; the scope fence allows the agent's own folder and rejects other agents' folders; private files stay in `.xherd/`. Folder names are agent names, so parallel agents never collide. xherd may suggest a `.gitattributes` line that collapses these files in PR diffs (never added silently). |

## 8. Pluggable SDD: flow packs, step roles, dynamic flows

### 8.1 Principle

The daemon knows **stages, gates, tasks, commits and PR rounds** in the abstract. A flow pack maps its own documents and commands onto them, and the agent declares each transition with XAP. The daemon validates each declaration and either accepts it or refuses with a message the agent can act on.

### 8.2 Flow packs

| ID | Requirement |
|---|---|
| R-201 | A flow pack is a directory or git repo with `xherd-flow.yaml` (schema in design §6). |
| R-202 | The manifest declares: name, version, XAP protocol version, flows, each flow's stages with **role**, gate, autonomy class and **next** command (with a small fixed set of conditions), and the `start`, `resume`, `handoff`, `status` commands. |
| R-203 | Install targets per runtime (paths, command prefix, format: command, skill, workflow, agent). xherd records the hash of each installed file; upgrades replace unchanged files and keep files the user edited (writing the new version next to them and warning). |
| R-204 | Optional JSON schemas for document frontmatter, checked by `xherd gate open`. |
| R-205 | Optional `parse` command that turns the pack's files into the normalised state (R-213). Without it, the pack keeps state current through XAP calls or uses xhSpec formats. |
| R-206 | Docs location: private by default; a profile may opt in to a committed path; a pack that only works with committed files declares `requiresCommitted: true` and xherd warns when the repo has not opted in. |
| R-207 | `xherd flow test <dir>`: manifest schema, prompt rendering per runtime, parser output on fixtures, and a fake-runtime run of each flow through its XAP calls. |
| R-208 | Conformance levels unlock UI features: L0 status only, L1 gates, L2 tasks + commits, L3 PR rounds, L4 stage hooks + ledgers. |
| R-209 | A pack never needs daemon code changes; missing capabilities mean a new XAP version. |

### 8.3 Step roles

| ID | Requirement |
|---|---|
| R-210 | Canonical roles: `explore`, `spec`, `design`, `tasks`, `implement`, `review`, `ship`, `pr`. Each stage declares its role. Per-step settings (runtime, model/tier, effort, driver), autonomy defaults, Kanban stage cards and icons are keyed by role, so a setting such as "review on Codex" applies to every pack. |
| R-211 | Several stages may share a role. A stage without a fitting role uses `custom` and declares its class; per-step settings fall back to the agent's default. |
| R-212 | `xherd flow test` fails when a stage has no role and warns when a pack lacks `spec`, `implement` or `ship`. |

### 8.4 Normalised state

| ID | Requirement |
|---|---|
| R-213 | Whatever the pack, the daemon and UI work with: current stage; stages with status (pending, active, gated, approved, skipped); the gate; tasks; commits; PR items; open questions; issues; warnings; and free-form `extra.*` data from the pack's parser for custom views (§11). |

### 8.5 Stage hooks

| ID | Requirement |
|---|---|
| R-401 | Profiles and packs may declare hooks per role and event: `pre.<role>`, `post.<role>`, `gate.<role>`, `pr.opened`, `pr.merged`, `agent.created`. |
| R-402 | Hook kinds: `command` (argv in the worktree, timeout, `XHERD_*` env), `agent` (a one-shot read-only prompt in the agent's runtime), `builtin` (`notify`, `tracker.transition`, `label`). |
| R-403 | `blocking: true` on `pre.<role>` makes the hook a tripwire for that stage's gate; declared artifacts (globs) are attached to the gate. |
| R-404 | Hook output goes to the agent log with secrets redacted; non-blocking failures only notify. |

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
| R-221 | Sessions run with `XHERD_ENV=1`, `XHERD_BIN`, `XHERD_AGENT_ID`, `XHERD_AGENT_TOKEN`. |
| R-222 | Registering a dynamic flow needs a name, the start command template (`/sdd-spec {prompt}`) and the allowed command prefixes (`/sdd-`). |
| R-223 | `gate open … --next "<command>"` tells xherd what to send after approval; it must start with an allowed prefix. Without it xherd sends the resume command or asks the user. |
| R-224 | `--role` maps the SDD's stage to a step role; missing → `custom`, class `critical` (always waits) until the user changes it. |
| R-225 | Optional calls (tasks, commits, PR, handoff, issues) can be adopted one at a time. |
| R-226 | Calls never break the SDD: exit 0 ok, 2 refused (fix and retry), 3 daemon unreachable (stop and wait); report-only calls queue offline. |
| R-227 | xherd ships `doc/integrate-your-sdd.md` with snippets for prompt-based and script-based SDDs. |
| R-228 | `xherd sdd check <dir>` reports the level an SDD reaches; `xherd sdd try <dir>` runs a fake agent through its calls. |
| R-229 | Compatible SDDs use the GitHub topic `xherd-sdd` and appear in the catalogue. |

Example block for a prompt-based SDD command:

```markdown
## When running under xherd
If the environment variable `XHERD_ENV` is `1`, after writing the spec run
`"$XHERD_BIN" gate open spec --role spec --doc <spec path> --next "/sdd-task <spec path>"`
then stop and wait. If it exits with code 2, fix what it says and run it again.
```

## 9. Pull-request follow-up

| ID | Requirement |
|---|---|
| R-301 | After ship, the agent's session ends; status `watching` when the PR is open and nothing is left to do. |
| R-302 | Without a declared PR URL, the host plugin's `pr-for-branch` finds it; for flow `none` the user may paste a link. |
| R-303 | Every `prPollMinutes` (default 5): PR status (state, number, mergeable, reviewers), checks, and comments through host plugin commands. |
| R-304 | New comments (ids not seen, not written by xherd, recognised by a hidden marker in the body) are saved to `pr-comments.json` and the agent is queued with the PR review command (fresh session unless off). Flow `none`: notify only. |
| R-305 | CI: notify when checks start failing and when all pass again. Merge conflicts: notify once. |
| R-306 | PR decisions: the user decides resolve/skip (with a note) per comment; xherd writes the decisions and sends the apply command. Replies: approved replies are posted by the host plugin (with the marker), or by the agent for hosts without a reply command. |
| R-307 | Merged or closed: follow-up task done, stage `done`, status `done`, `pr.merged` hooks run. |
| R-308 | Ticket updates (comment, transition) happen only through the agent after the user approves `pr.md`, or through opt-in stage hooks. |

## 10. Plugins

### 10.1 Kinds, manifest, protocol

| ID | Requirement |
|---|---|
| R-501 | Kinds: `runtime`, `tracker`, `host`, `flow`, `notifier`, `toolchain`, `panel`. |
| R-502 | Manifest `xherd-plugin.json` (packs: `xherd-flow.yaml`) with `name`, `kind`, `version`, `min_xherd_version`, `label`, `description`, `settings` (key, label, type, default, secret, required), kind-specific `data`, prompt `instructions` fragments, `commands`, `hooks`, `checks` (argv, fix, optional; pass = exit 0 within 10 s), `build` (install-time), `timeoutSeconds` (default 60). |
| R-503 | Commands are argv in any language (no shell); `./x` resolves inside the plugin folder. stdin carries `{command, input, settings, secrets, agent}`; stdout returns JSON. Non-zero exit, timeout (process tree killed), bad JSON or wrong shape = failure, shown on the agent and in Setup checks; stderr goes to the log with secrets redacted. |
| R-504 | Built-in plugins (GitHub, GitLab; Claude Code, Codex, opencode; SDKMAN, nvm, uv, Go, rustup; xhSpec) are compiled into the binary behind the same interface; an external plugin with the same name overrides one. |
| R-505 | **The CLI is the plugin API**: commands run with `XHERD_BIN`, `XHERD_URL`, a plugin-scoped `XHERD_PLUGIN_TOKEN`, `XHERD_PLUGIN_ID`, `XHERD_PLUGIN_ROOT`, `XHERD_PLUGIN_CONFIG_DIR`, `XHERD_PLUGIN_STATE_DIR` and `XHERD_CONTEXT_JSON`, and may call any `xherd` command their token allows. No SDK. |
| R-506 | Settings resolution: manifest default → Settings → profile override (non-secret only). Secrets are global, stored in `secrets.json` (mode 0600), never sent to the UI (it only learns which are set), passed only on stdin to that plugin. |
| R-507 | A broken plugin is listed with its error but cannot be chosen. |

Host commands: `pr-for-branch {branch} → {url|null}`, `pr-status {url} → {state, number, url, mergeable?, reviewers?}`, `pr-checks {url} → [{name, bucket, link, workflow}]`, `pr-comments {url} → [{id, kind, author, body, url, createdAt, path?, line?, inReplyTo?}]`, `pr-reply {url, to, body} → {}`, `pr-template {} → [{path, name, body}]`. Runtime command: `usage {session, cwd, payload} → {input, output, cacheRead, cacheWrite, context}`. Event hooks receive `{event, agent, ticket, settings, secrets}`; output ignored.

### 10.2 Runtime drivers

| ID | Requirement |
|---|---|
| R-511 | A runtime plugin declares the drivers it supports: `terminal` (launch/resume argv, command prefix, Enters after slash commands, permission modes, hook files, trust files, screen rules) and optionally `acp` (agent argv, session resume support). No `acp` entry = no ACP support; the UI shows a badge for runtimes that have it. |
| R-512 | The driver is chosen per agent, per profile or globally (most specific wins); only supported drivers are offered. It applies at the next session start. |
| R-513 | `terminal`: live terminal in the browser; the user can type into it. |
| R-514 | `acp`: xherd is the ACP client; prompts are exact messages; the UI shows a transcript; permission requests become `blocked` with Allow / Allow always / Deny (also on phones); "Take over in terminal" resumes the session with the terminal driver where the runtime allows it. |

### 10.3 Models and effort per step

| ID | Requirement |
|---|---|
| R-521 | Runtimes list models with a tier (`high`, `medium`, `low`) and how to pass them; optionally effort levels. |
| R-522 | Per-step configuration = runtime + model or tier + effort + driver, for each step role, at agent, profile and global level; unset values fall back to the next level, then the runtime default. |
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
| R-545 | Plugins run with the daemon's rights (minus what their scoped token forbids); there is no plugin sandbox in v1, and the confirmation says so. |
| R-546 | A plugin used by a live agent cannot be disabled or removed until the agent is paused or done. |
| R-547 | `xherd plugin new --type <kind> --name <name> [--dir] [--lang rust|go|node|python|bash]` (interactive without flags) creates a working skeleton: manifest with all fields for the kind, one example command per protocol command returning valid output, README, fixtures, test config and a CI workflow. Right after, `xherd plugin test` passes and `xherd plugin link` makes it usable. `xherd flow new` scaffolds a pack (stages with roles, prompts per runtime, schemas, default views). |
| R-548 | CLI kit: `xherd plugin new|test|link|unlink|ls|search|install|update|remove`. `plugin test` checks a plugin's conformance (outputs validated; never runs reply/posting commands or hooks). |

## 11. Declarative views

Screens are built from **panels** arranged by declarations. xhSpec's own agent page is declared the same way.

| ID | Requirement |
|---|---|
| R-1101 | Declarable: agent page (header fields, gate texts per stage, rail sections and panels, actions), New agent inputs (JSON Schema → generated form; values reach the agent as `{{inputs.x}}`), Needs-you texts per gate, Kanban cards (record source, title, column mapping). |
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
| R-601 | The recommended install is `docker compose up -d` with the published multi-arch image (amd64, arm64). One shared container runs the daemon and every agent; it contains `xherd`, `git`, `tmux`, `gh`, `glab`, `openssh-client`, the agent CLIs and the version managers (§12.4). |
| R-602 | **Same-path workspace**: the workspace root (default `~/xherd` on the host) is mounted at the identical absolute path in the container, so repos, worktrees, `.git` pointers and paths shown in the UI are valid on both sides. |
| R-603 | The container runs agents as the host user's UID/GID so files keep the user's ownership. |
| R-604 | Runtime logins, xherd data, toolchains and caches live in a named volume (`xherd-home`). |
| R-605 | Repos outside the workspace are rejected with the mount to add (`XHERD_EXTRA_PATHS`, also same-path) or a "clone into workspace" option. |
| R-606 | `xherd docker init` (host) writes the compose file and `.env` for the detected engine; `xherd docker doctor` checks same-path mounts, ownership, host-side file changes seen in the container, SSH agent, and port binding to `127.0.0.1`, each with a fix. |
| R-607 | The native binary (host tmux) is supported with the same behaviour. |
| R-608 | Each agent gets `XHERD_PORT_BASE` (a free block of 10 ports) so dev servers of parallel agents do not collide. |

### 12.2 Container engines on macOS

Supported: Docker Desktop (reference), and OrbStack, Colima, Podman Desktop, Rancher Desktop when `xherd docker doctor` passes. **Experimental**: [Kiri](https://runkiri.com/) (Docker-compatible runtime, beta) and Apple's `container` with [opossum](https://github.com/suruseas/opossum) (Compose subset, macOS 26+, no Docker API). xherd does not install an engine.

### 12.3 Git access: HTTPS and SSH

| ID | Requirement |
|---|---|
| R-681 | Remote URLs are used as they are; one install may mix HTTPS and SSH per host. |
| R-682 | SSH: the host's SSH agent forwarded into the container (Linux `$SSH_AUTH_SOCK`, Docker Desktop `/run/host-services/ssh-auth.sock`, exposed to the container user through a proxy owned by that user), or mounted/secret key files referenced in an xherd-managed SSH config include. |
| R-683 | Host keys: GitHub, GitLab and Bitbucket keys pre-seeded; other hosts' fingerprints shown in the UI for the user to accept. |
| R-684 | HTTPS: tokens through a git credential helper (`gh`, `glab`, or `xherd credential`), never in URLs or `.git/config`; optional CA bundle per host. |
| R-685 | Settings → Git access per host: protocols, credential source, username, CA; optional `preferHttps`/`preferSsh` rewrite (off by default). Host plugins reuse the token. |
| R-686 | Welcome step "Git access": use the SSH agent, log in with GitHub's device flow (`gh auth login --web`, code shown in the UI), or paste a token; each ends with a live test of git transport and API access. |
| R-687 | `xherd git doctor [host] [--repo URL]` checks, in order: agent socket readable, key loaded (`ssh-add -l`), host key known, credential helper configured, token valid with the needed permissions, organisation SSO authorised, OAuth app allowed by the organisation, git vs API access separately. It prints the first failure with its fix and never prints secrets. |
| R-688 | An auth failure during an agent's push or a host command blocks the agent with reason "git access" and shows the doctor result. |

### 12.4 Toolchains in the shared container

| ID | Requirement |
|---|---|
| R-651 | The image ships version managers, not language runtimes (except what the agent CLIs need). Languages are installed into `xherd-home`. |
| R-652 | Before an agent starts, xherd reads the worktree's version files (`.sdkmanrc`, `.nvmrc`, `.node-version`, `.python-version`, `pyproject.toml`, `go.mod`, `rust-toolchain.toml`), installs missing versions with the matching toolchain plugin, and starts the runtime with that `PATH`. Each worktree gets its own versions; no global "current version" is switched. |
| R-653 | Welcome step "Toolchains" and Settings → Toolchains: languages with version pickers (defaults Java 21 + Maven + Gradle, Node LTS + pnpm, Python 3.12 + uv, Go, Rust stable), install with a live log, skip allowed. Native installs only detect, never install on the host. |
| R-654 | Profiles may declare `toolchain:` versions for repos without version files and `setup:` commands run once per new worktree. |
| R-655 | Setup checks show whether each profile's toolchain resolves, with "Install missing". |

### 12.5 Deployment modes

| Feature | Local Docker | Native | Server (Docker) | Kubernetes |
|---|---|---|---|---|
| Agents, gates, UI, PR follow-up, phone | ✓ | ✓ | ✓ | ✓ |
| Web terminal; `xherd attach` over websocket | ✓ | ✓ | ✓ | ✓ |
| VS Code in the browser | ✓ | ✓ | ✓ | ✓ |
| Local apps via URL schemes | ✓ | ✓ | – | – |
| VS Code Remote-SSH / JetBrains Gateway | – | – | ✓ (SSH to host) | via SSH sidecar (**verify**) |
| Dev Containers attach | ✓ | – | via Docker context over SSH (**verify**) | – |
| Host commands (companion) | ✓ | ✓ | – | – |

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
| R-703 | Host companion (`xherd` on the host, client mode): `xherd attach <agent>` (over the daemon's terminal websocket), `xherd open <agent> [--app NAME]`, `xherd shell <agent>`, `xherd login [URL]`. |
| R-704 | Optional host bridge (`xherd companion`, later built into the desktop app): runs only the configured Open-in commands with `{dir}` substituted, for requests authenticated by the daemon. |
| R-705 | VS Code in the browser (openvscode-server) inside the container, for remote devices. |

## 13. Guardrails and security

### 13.1 What is protected

The host machine; git and model credentials; plugin secrets; the integrity of repos (base and protected branches); the user's money (tokens, paid APIs); and the user's attention (no silent actions in their name).

### 13.2 Threats and guardrails

| # | Threat | Guardrails |
|---|---|---|
| T1 | **Prompt injection** from tickets, PR comments, web pages or repo files steering an agent to run harmful commands, leak secrets or push bad code | Untrusted text is passed to agents inside clearly marked blocks with an instruction to treat it as data (R-1201); PR triage and replies always wait for a human; autonomy limits and tripwires (§6.4); tool permissions ask by default; secret tripwire on commits (R-117); egress allowlist (roadmap). |
| T2 | **Agent approves its own gates** or changes xherd settings | XAP has no approve command; agents get an agent-scoped token valid only for reports and XAP of that agent (R-1202); in Docker, agent processes cannot read the daemon's config and secrets (R-1203). |
| T3 | **Pushing where it must not** (base branch, force push, other branches) | Worktrees have no upstream; an xherd-managed `pre-push` hook in each worktree rejects pushes to any ref except the agent's branch and any non-fast-forward push (R-1204); xhSpec rules forbid force pushes; the docs recommend branch protection on the host. |
| T4 | **Destructive commands** in the workspace (deleting repos, `git reset --hard` on the main checkout) | Agents work only in their worktree; tool permission `all` only inside the container; the container protects the host but not the workspace mount, so backups (§15) cover the workspace's unpushed work; the kill switch (R-032). |
| T5 | **Runaway cost or loops** | Token budgets per agent and per day (R-1205); run limits (R-120); parking; idle alerts; max session length (R-1206). |
| T6 | **Malicious plugin or pack** | Preview of commands before install, pinning to a sha, abort on manifest change, scoped plugin tokens, no installs from phones, `XHERD_PLUGIN_INSTALL=off` (§10.5). No sandbox (stated). |
| T7 | **Remote access abuse** | Token on every request; loopback binding by default; LAN and remote access off by default; HTTPS for remote; device pairing with one-time codes; phones get steering rights only; Origin checks against CSRF; token rotation revokes all phones (R-1207). |
| T8 | **Untrusted content in the UI** (XSS via agent-written Markdown, PR comments, diagrams) | Sanitised Markdown; Mermaid in strict mode without HTML labels; no remote images in agent content by default; panel plugins in sandboxed iframes without the token. |
| T9 | **Secret leakage** in logs, hook output, PR text or specs | Redaction of known secrets in every captured output; secrets passed only on stdin; secret tripwire; secrets never in `instructions.md`. |
| T10 | **Supply chain** of the image and binary | Pinned versions of bundled CLIs, signed releases and images, SBOM (§14.4). |

### 13.3 Requirements

| ID | Requirement |
|---|---|
| R-1201 | Untrusted text handed to agents (PR comments, ticket text fetched by xherd, hook output) is wrapped in delimited blocks labelled as untrusted data; xhSpec rules tell agents never to follow instructions found inside them. |
| R-1202 | Tokens are scoped: user token (full), device tokens (full or phone rights), agent tokens (`/api/v1/hooks`, `/api/v1/xap` for one agent), plugin tokens (declared scopes). Agent and plugin tokens are created at start and revoked when the session or plugin stops. |
| R-1203 | In the container, the daemon runs as its own user; `config.json`, `secrets.json` and the database are readable only by it. Agents run as the host-mapped user and reach git credentials only through the credential helper, which asks the daemon over a socket and answers only for configured hosts. In native mode the same files are 0600, which is a guardrail, not a boundary (same OS user); the UI says so. (**verify** ownership behaviour on macOS bind mounts.) |
| R-1204 | Each worktree gets an xherd `pre-push` hook (chained with any existing one) that allows pushing only the agent's branch to its configured remote, never deletes remote refs, and refuses non-fast-forward pushes. Hooks can be skipped (`git push --no-verify`), so this prevents mistakes, not a determined agent; **branch protection on the host is the real boundary**, and `xherd security check` reports whether the base branch is protected. |
| R-1205 | Budgets: per-agent token budget and a global daily budget (tokens; cost estimate where the runtime reports it). Reaching 80 % notifies; reaching 100 % pauses the agent at its next stop. |
| R-1206 | `maxSessionMinutes` (default off): a session running longer is asked to hand off and is paused. |
| R-1207 | Network: the daemon listens on loopback by default; LAN (phones) and remote hostnames are opt-in; other connections are dropped before HTTP. |
| R-1208 | Audit: approvals (ledger), XAP calls, plugin installs/updates, settings changes and token rotations are recorded as events and kept for `eventRetentionDays` (default 90). |
| R-1209 | Unattended mode (`never` autonomy) requires a one-time confirmation that lists what will happen without the user (draft PRs, no ticket updates), and is unavailable for `hotfix` flows. |
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
| R-1304 | **Draining**: restarting the container ends every tmux session, so `xherd docker update` (host) first asks live agents to hand off and pauses them (or waits until none is running, with a timeout and a "force" option), takes a backup (§15), pulls the new image, recreates the container, and resumes the paused agents (same runtime sessions where possible, else fresh sessions). Native updates replace the binary and restart only the daemon; agents keep running in tmux. |
| R-1305 | Database migrations are forward-only, numbered, run at start inside a transaction, after an automatic copy of the database. A failed migration leaves the old database in place and the daemon refuses to start with the error. Downgrades restore from that copy. |
| R-1306 | Config files carry a schema version and are migrated the same way; unknown keys are kept. |
| R-1307 | Built-in prompts (xhSpec) are re-installed on update with the install-manifest rules (R-203): user-edited files are kept and the new version is written next to them with a warning. |
| R-1308 | **In-flight agents keep their pack version**: an agent records the pack version it started with and uses it until done, unless the user upgrades it explicitly. |
| R-1309 | Plugins: updates are explicit (R-543); an update that requires a newer xherd is refused with the needed version. |
| R-1310 | Release notes list breaking changes, migrations and any manual step; the UI shows them after an update. |

### 14.3 Toolchains and agent CLIs

| ID | Requirement |
|---|---|
| R-1311 | Agent CLIs bundled in the image are pinned per image version; Settings shows their versions, and a check warns when the installed CLI is outside the tested range of its runtime plugin. Users can update a CLI inside the container (kept in `xherd-home`) at their own risk. |
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
| Database, config, secrets, devices | `<data>` in `xherd-home` | ✓ (secrets encrypted) | Agents, profiles, approvals, settings |
| Plugins and packs | `<data>/plugins/` | ✓ | Reinstalling may fetch different code |
| Archived `.xherd/` folders | `<data>/archive/` | ✓ | History of finished agents |
| Live agents' `.xherd/` folders | worktrees | ✓ | Specs, ledgers and handoffs are not in git |
| Unpushed work | worktrees | ✓ as git bundles of agent branches + patches of uncommitted changes | Not on the remote yet |
| Runtime logins | `xherd-home` (`~/.claude`, `~/.codex`, `~/.config/gh`) | optional (encrypted) | Re-login is easy but annoying |
| Toolchains, package caches, clones | `xherd-home`, workspace | ✗ | Reproducible |

### 15.2 Requirements

| ID | Requirement |
|---|---|
| R-1401 | `xherd backup create [--to <dir>] [--include-logins]` writes one archive: a consistent database copy (SQLite online backup), config, plugins, archives, live `.xherd/` folders, git bundles of agent branches with commits not on the remote, and patches of uncommitted changes. Secrets and logins are encrypted with a passphrase or excluded. |
| R-1402 | Scheduled backups (Settings → Backups): daily when a destination is set, keep N (default 7), to a host-mounted folder; an optional `command` destination (e.g. `rclone copy`, `restic backup`) runs after each backup. |
| R-1403 | `xherd backup restore <archive>` (daemon stopped): restores data and plugins, re-creates `.xherd/` folders and agent branches from bundles in existing worktrees (or lists what could not be restored), and marks restored live agents `paused` so the user resumes them deliberately. |
| R-1404 | `xherd backup verify <archive>` checks integrity (hashes, database check, bundle verification) without restoring. |
| R-1405 | Updates (R-1304) and migrations (R-1305) always take a backup first; the last 3 of these are kept separately. |
| R-1406 | Kubernetes: the Helm chart documents PVC snapshots for the home and workspace volumes in addition to `xherd backup`. |
| R-1407 | Export/import of profiles and settings as JSON (without secrets) for moving to another machine. |

## 16. Supporting features

### 16.1 Repo profiles

Matched by remote URL, then path. Fields: name, colour, match; commands (run, test, build, lint, extras; each runs in its own tmux window from the agent page and can be stopped); extra instructions; untracked files (copy/symlink); base/staging/hotfix branches; default flow and pack; ticket prefix; autonomy, sensitive paths, max diff lines; tool permissions; runtime/model/driver per step; plugin setting overrides; stage hooks; toolchain and setup; spec folder mode; budgets. Export/import as JSON.

### 16.2 TODOs

Reminders (due at a time, once or repeating) and **condition checks**: at each due time xherd starts a short read-only session of a chosen runtime and model, with a system prompt that forbids changes and asks for a final line `XHERD_RESULT {"met": bool, "summary": "…"}`. Met → needs you; not met → rescheduled; a question or permission prompt, or 2 minutes of silence → needs you (terminal shown); failure → error. Actions: complete, reschedule, check now, edit, delete.

### 16.3 Notifications

Browser notifications (permission asked on the first click), the desktop app later, OS notifications from a native daemon when no browser tab is open and a desktop exists, and **notifier plugins** (ntfy built in; Telegram and Slack webhooks as plugins). Triggers: gate opened, auto-approved, blocked, delivery failed, idle alert, new PR comments, CI failed / green again, conflicts, budget thresholds, TODOs. One notification per agent and kind replaces the previous one.

### 16.4 Work log and tokens

Agent time (time in `running`) and the user's time on an agent page, per day and ticket. Token usage per runtime session from the runtime's `usage` command or the stop payload; totals per agent; cost estimate where known.

### 16.5 `instructions.md`

Rewritten on every start, resume and autonomy change: workspace notes (folder agents), autonomy level and what it means for the agent, ticket key and link with the tracker's fragment, git host fragment, repo commands (or a note to use the project's usual ones and state them in evidence), repo instructions from the profile, the XAP commands the agent must call, its port block, and the untrusted-content rule.

### 16.6 Memory proposals

The review step may propose updates to the repo's `CLAUDE.md` / `AGENTS.md` (full proposed text plus a reason; a starter `AGENTS.md` when none exists). The UI lets the user save (optionally editing first; the agent then commits it through a normal commit gate) or dismiss (dismissing a new file is remembered per repo). Pending proposals do not block the PR.

## 17. User interface

### 17.1 Principles

- One colour means **"needs you"** and is used for nothing else; actions, running, errors and done each have their own tone.
- Dense, calm layout for many agents at once; monospace for names, branches, commits and paths.
- View state lives in the URL (bookmarkable; Back works).
- In-app dialogs for errors (long, copyable messages) and confirmations; never browser `alert`/`confirm`.
- Agent-written and host content is untrusted (§13 T8).
- Light and dark themes; reduced motion respected; visible focus; reachable by keyboard.
- Everything live over one websocket; a reconnect reloads the full state.
- English UI in v1, strings kept in one catalogue for later translation.

### 17.2 Screens

| Screen | Content |
|---|---|
| **Overview** | "Needs you" first (longest wait first: what is waiting and the one useful action), then every agent: status, stage, task progress, branch, tokens, last activity; filters. |
| **Agent page** | Header (name, ticket link, status, autonomy, runtime, tokens, actions); gate box (what to approve, held reason, assumptions, the agent's reply, Approve / Request changes, review changes for commits); message box; rail of panels from the view declaration (terminal or transcript, documents with "changed since reviewed", tasks, commits, diff with "viewed" marks, PR, issues, memory, log); profile commands; Open in. |
| **To review** | Auto-approved items with reason, assumptions, diff since last review, Mark seen. |
| **Kanban** | Tasks of all agents plus stage cards and TODOs; filters and swimlanes in the URL. |
| **New agent / New TODO** | Generated form including pack inputs; ticket link preview; repo picker or clone; branch list. |
| **Settings** | General, Notifications, Open in, Git access, Toolchains, Repo profiles, Plugins (install, settings, ACP badges), Runtime per step, Backups, Remote access. |
| **Setup checks / Welcome** | First-run wizard: where it runs → tools (runtime, tracker, host) → git access → toolchains → checks → ready. Until required checks pass, other screens are disabled. |
| **Work log** | Agent time and user time per day and ticket. |

### 17.3 Phones

Phones (LAN with a token link or QR code, optional HTTPS with a local CA) get **steering rights**: view everything except settings and terminals; approve, request changes, answer, decide PR comments, approve replies, pause, resume, add notes, handle TODOs, answer ACP permission prompts. No terminals, commands, settings, plugin installs or new agents.

## 18. CLI

```
xherd [start] [--no-open]               start the native daemon if needed and open the UI
xherd daemon                            run the daemon in the foreground
xherd stop | restart | status
xherd service install|uninstall         systemd user unit / launchd agent
xherd docker init|doctor|update         host: compose setup, checks, drained update
xherd login [URL]                       pair this CLI with a daemon (host companion)
xherd new <KEY> [--slug s] [--flow f] [--pack p] [--repo dir|url] [--base b]
          [--runtime r] [--autonomy a] [--tool-permissions p] [--driver terminal|acp] ["prompt"]
xherd ls | attach | open | shell | pause | resume | approve | cancel <agent>
xherd changes <agent> "comment"         request changes at the open gate
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
xherd companion                         host bridge for Open-in commands
```

`<agent>` is a name or a unique ticket key.

## 19. Non-functional requirements

| ID | Requirement |
|---|---|
| R-901 | `xherd hook` and `xherd report` finish in < 10 ms when the daemon is up, never print to stdout, never exit non-zero. |
| R-902 | 20 live agents and 200 total stay responsive (API p95 < 100 ms on a laptop). |
| R-903 | Single static binary with the UI embedded; image size kept small (target < 400 MB without languages, agent CLIs included, **verify**). |
| R-904 | All state survives daemon restarts; agents survive daemon restarts (not container restarts, see R-1304). |
| R-905 | Structured JSON logs; optional OpenTelemetry export (traces around reports, delivery, plugin commands). |
| R-906 | No telemetry. The only outbound call made by xherd itself is the optional update check. |
| R-907 | Supported platforms: Linux and macOS (native and Docker); Windows through WSL2 or Docker. |

## 20. Beyond coding (vision)

The core (sessions, stages, gates, autonomy, per-step models, XAP, ledgers, views) is not specific to code. Three seams keep other work possible: **workspaces** (`git-worktree`, `folder`, later read-only PR checkouts), **artifacts** (any file type, approvable by hash, shown by viewer panels) and **delivery** (PR today; docs or media publishing later).

| Flow | Effort on top of v1 |
|---|---|
| Review someone else's PR (read-only checkout, draft comments ✋, post) | Low |
| Research and write a design (folder, web/MCP tools, outline ✋, draft ✋, publish) | Low–medium |
| Prepare and create a video (script ✋, storyboard ✋, rough cut ✋, render, upload) | High: media viewers, long jobs, large files, paid APIs |

| ID | Requirement |
|---|---|
| R-1001 | Workspace kinds behind one interface; git features shown only when the workspace supports them. |
| R-1002 | The agent page is built from workspace and pack capabilities (§11). |
| R-1003 | Packs declare artifacts per stage; gates may point at any artifact. |
| R-1004 | Ticket keys are optional for non-coding flows. |
| R-1005 | Later: `delivery` plugins, tracked **jobs** for long renders, budgets covering paid tool APIs. |

## 21. Roadmap

1. **v1.0**: MVP scope (§2.3).
2. **v1.x**: Codex and opencode runtimes; Kanban, To review, Work log; Welcome toolchain installer and toolchain plugins; plugin install from the UI and catalogue; TODOs; `never` autonomy.
3. **ACP driver** per runtime, selectable per agent.
4. Server and Kubernetes deployment; VS Code Remote-SSH entries.
5. **Electron desktop app** with tray, notifications and built-in host companion.
6. Panel plugins; declarative views for external packs; PR-review flow pack; research/design flow pack.
7. Per-agent containers with egress allowlists; parallel task execution inside one agent (sub-worktrees, different models per task).
8. Multi-repo tickets.

## 22. Decisions

| # | Decision |
|---|---|
| D-1 | Daemon in **Rust**, one static binary with the web UI embedded. |
| D-2 | **Docker-first** with same-path workspace mounts; native binary supported; server/Kubernetes with reduced Open-in. |
| D-3 | **One shared container** with all tools in v1; version managers for languages. |
| D-4 | **Single user** per daemon. |
| D-5 | SDD documents **private** in `.xherd/` by default; committing is opt-in per profile. |
| D-6 | Approvals by **content hash** in an append-only ledger; the daemon never edits agent documents. |
| D-7 | Pluggable SDD: flow packs, step roles, and dynamic flows via XAP. |
| D-8 | Driver chosen by configuration: live terminal (default) or ACP. |
| D-9 | Runtimes declare models with tiers and effort; steps choose runtime + model/tier + effort + driver. |
| D-10 | Plugin kinds include `toolchain` and `panel`; the CLI is the plugin API; plugins installable from the UI, pinned and previewed; `xherd plugin new` scaffolds them. |
| D-11 | **Declarative views** with a fixed vocabulary; no visual workflow builder. |
| D-12 | The repo's PR template takes priority. |
| D-13 | Git over HTTPS and SSH, per host, with guided setup and a doctor. |
| D-14 | Desktop app in **Electron**, post-v1. |
| D-15 | Default port **4740**. |
| D-16 | **MIT** licence. |
| D-17 | Not pursued: agents controlling other agents, a combined view of several daemons, replacing tmux with another terminal server. |
