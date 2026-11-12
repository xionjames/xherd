# XAP — xherd agent protocol (reference)

> Status: **draft, protocol version 1** · 2026-11-06 · Part of [`spec.md`](../internal/spec.md) (§5.4, §6, §8).
>
> XAP is how an agent, an SDD's commands, or a wrapper tells xherd what is happening: status, gates, tasks, commits, PRs. It is a set of `xherd` subcommands (and the same operations as MCP tools). The daemon validates every call; agents never approve anything.

## 1. Conventions

### 1.1 Environment

Every agent session runs with:

| Variable | Meaning |
|---|---|
| `XHERD_ENV` | `1` when running under xherd. SDDs skip XAP calls when it is not `1`. |
| `XHERD_BIN` | Absolute path of the `xherd` binary. Call XAP as plain `xherd …` (its folder is first on the session `PATH`; prefer it over the variable, which runtimes cannot check and ask about every time). |
| `XHERD_AGENT_ID` | The agent's id. |
| `XHERD_AGENT_TOKEN` | Agent-scoped token; valid only for this agent's reports and XAP calls. |
| `XHERD_URL` | Daemon URL (default `http://127.0.0.1:4740`). |
| `XHERD_PORT_BASE` | First of 10 ports reserved for this agent. |

If `XHERD_AGENT_ID` is missing, the CLI looks for `.xherd/agent.json` in the current directory and its parents.

### 1.2 Exit codes

| Code | Meaning | What the agent should do |
|---|---|---|
| 0 | Accepted (or already in that state) | Continue |
| 2 | Refused: the call is invalid for the current state or files | Read the one-line message on stderr, fix it, call again |
| 3 | Daemon unreachable (for calls that need an answer) | Stop and wait; do not retry in a loop |
| 4 | Not running under xherd (`XHERD_ENV` unset and no `agent.json`) | Skip; the SDD works standalone |
| 64 | Usage error (bad flags) | Fix the command |

Report-only commands (`report`, `hook`, `stage enter`, `task start|done|block`, `issue add`) never fail because of the daemon: when it is down they are queued to `hook-queue.jsonl` and exit 0.

Refusals any command may return: `not_live` (the agent has no live session), `no_flow` (stage, gate, task and commit calls for an agent with flow `none`; `ask`, `pr opened`, `handoff written` and `issue add` still work), `no_workspace`, `unknown_agent`, `usage` (malformed call).

### 1.3 Output

Plain text by default (one line on success). `--json` prints `{"ok": true, "result": {...}}` or `{"ok": false, "code": "...", "message": "..."}`. Refusal codes are stable strings (see each command).

### 1.4 Idempotency and ordering

- Calls with the same arguments in the same state are no-ops (exit 0).
- Every call carries a client-generated id (dedupe on replay) and, for status reports, a sequence number (`--seq`, monotonic per agent; the CLI generates it from a counter in `.xherd/seq` when omitted).
- The daemon records every accepted call as an event `xap.<command>`.

### 1.5 Paths

Document paths are relative to the agent's spec folder (`.xherd/spec/` by default, or the committed folder) unless they start with `./` (relative to the worktree root). Absolute paths outside the worktree are refused (`path_outside_workspace`).

## 2. Status

### `xherd hook <event>`

Called by runtime hooks; reads the runtime's hook JSON on stdin. Events: `session-start`, `user-prompt-submit`, `post-tool-use`, `notification`, `stop`. **Never prints to stdout, never exits non-zero.**

### `xherd report`

```
xherd report --state working|idle|blocked [--message TEXT] [--session ID] [--resume-cmd CMD] [--seq N]
xherd report --release [--seq N]
```

For agents or wrappers without hooks. `--resume-cmd` is the command that resumes this session after a restart. `--release` clears the agent's runtime state when the agent exits.

## 3. Stages and gates

### `xherd stage enter <stage> [--role ROLE]`

Marks the current stage (UI and per-step accounting). For dynamic flows, an unknown stage id is appended to the flow.

### `xherd gate open <stage> [options]`

```
xherd gate open <stage> [--role ROLE] [--doc PATH]... [--artifact PATH]...
                [--next COMMAND] [--needs-human REASON] [--question TEXT]...
```

Declares that the agent stops for approval of the listed documents/artifacts. The status becomes `waiting-input` at the next stop.

| Option | Meaning |
|---|---|
| `--role` | Step role (`spec`, `design`, `tasks`, `implement`, `review`, `ship`, `pr`, `explore`, `custom`). Required for dynamic flows; packs already know it. |
| `--doc` / `--artifact` | Files to approve; each is hashed at declaration. |
| `--next` | Dynamic flows: command to send after approval; must start with an allowed prefix. |
| `--needs-human` | Forces a human approval (tripwire) with this reason. |
| `--question` | Extra open questions (repeatable). For packs whose documents have a `## Open questions` section (the xherd pack), the questions are read from the documents and this option is only for a question that has no place in them; one that matches a list item is not repeated. |

Special stages: `pr-triage` and `pr-replies` (PR rounds, no documents).

Without `--doc`/`--artifact`, the stage's documents from the pack are used.

Refusals: `unknown_stage`, `no_docs_gate` (the stage gates commits or has no documents), `doc_missing`, `doc_schema` (frontmatter does not parse or fails the pack's schema; message names the field), `already_approved` (same content already approved), `path_outside_workspace` (also through symlinks), `next_not_allowed` (`--next` on a pack flow: the pack decides), `gate_open_commit` (a commit proposal is still open), `memory_invalid` (xherd flows: `.xherd/spec/memory.md` is missing, lacks one of the sections State, Decisions, Code map, Watch out, or has more than 80 lines; only structure is checked; `commit propose` refuses it too).

### `xherd ask <text>`

A question without documents. Status becomes `waiting-input`; the user's answer is typed back to the agent.

## 4. Tasks

```
xherd tasks sync [--file PATH]
xherd task start <ID>
xherd task done <ID> --evidence TEXT
xherd task block <ID> --reason TEXT
```

`tasks sync` re-reads the task list (built-in parser or the pack's `parse`); default file `tasks.md`. The file is the source of tasks; a status already reported through XAP is kept when the file still says `todo`. Refusals: `doc_missing`, `doc_schema` (the list does not parse), `task_unknown`, `task_not_ready` (dependencies not done), `task_already_doing` (another task is in progress), `evidence_missing`, `verify_failed` (`task done` only: see below), `verify_invalid` (`tasks sync` and `task done`: a `verify:` that does not parse as one shell command, checked with `sh -n` before the tasks are approved; write a single runnable command or none).

When the task has a `verify:` command, `task done` makes the daemon run it (`sh -c`, in the worktree, 10 minutes at most) before accepting. Exit 0: the task is done and its evidence is the agent's `--evidence` followed by `[verify exit 0]` and the last 20 lines of output. Non-zero exit or timeout: refused with `verify_failed` (one line: the command, how it ended, the last lines of output) and the task stays `doing`. A replayed call (same call id) does not run it again.

## 5. Commits

```
xherd commit propose <ID> --subject TEXT [--message TEXT] --files PATH[,PATH...] [--tasks T01,T02] [--needs-human REASON]
xherd commit done <ID> --sha SHA
xherd commit drop <ID> [--reason TEXT]
```

`propose` opens a commit gate; xherd computes the diff of exactly these files for review and runs the code tripwires. `--message` is the full commit message (`git commit -m`; the subject is used without it); the user can edit it at approval. **On approval xherd makes the commit itself** (stages exactly the proposed files, commits with the message, hooks included, never `--no-verify`), records its sha and marks the proposal done; the next command (`commit`) only records it in `commits.md`. If git refuses, the approval stands, git's output is written to `.xherd/commit-error.txt` and the agent fixes and commits (then `done`). `done` is accepted only after the proposal was approved, and checks that the commit exists on the agent's branch and touches exactly the proposed files. Proposing the same id again while it waits for approval replaces the proposal (after a change request). Commit ids are never reused once committed or dropped. Refusals: `commit_open` (another proposal is open), `docs_gate_open` (a document gate waits for approval), `commit_id_used`, `files_empty`, `task_unknown`, `files_outside_scope` (not in the `files:` of the listed tasks; no check when `--tasks` is absent or a listed task declares no files), `commit_unknown`, `commit_not_approved`, `commit_done` (already committed), `sha_unknown` (no such commit, or not on the agent's branch), `sha_files_mismatch`, `memory_invalid` (as for `gate open`).

## 6. Pull requests, handoff, issues, memory

```
xherd pr opened --url URL
xherd handoff written [--file PATH]
xherd issue add --title TEXT --severity minor|major|blocker [--about PATH]... [--body TEXT]
xherd memory propose --reason TEXT [--file PATH]
```

`pr opened` starts PR follow-up (refused with `bad_url` unless http(s)). `handoff written` confirms a pause/ship handoff (the pause waits for it); default file `.xherd/HANDOFF.md`, refused with `doc_missing` when absent. `issue add` appends to the issue ledger (`.xherd/ledger/issues.jsonl`). `memory propose` registers a proposed update of the repository's shared memory, `AGENTS.md`: `--file` (default `.xherd/AGENTS.proposed.md`) holds the whole proposed file, `--reason` one line on what it adds. One proposal per agent; proposing again replaces it. It never blocks the flow: the user accepts (xherd commits `AGENTS.md`, plus a `CLAUDE.md` with `@AGENTS.md` when that import is missing, on the agent's branch) or dismisses it. Refusals: `doc_missing`, `doc_schema` (empty, or over 64 KiB), `usage` (no reason), `memory_declined` (the repository has no `AGENTS.md` and the user dismissed a starter one before).

## 7. MCP transport

`xherd mcp` runs an MCP server over stdio exposing the same operations as tools: `xherd_gate_open`, `xherd_ask`, `xherd_stage_enter`, `xherd_tasks_sync`, `xherd_task_start`, `xherd_task_done`, `xherd_task_block`, `xherd_commit_propose`, `xherd_commit_done`, `xherd_commit_drop`, `xherd_pr_opened`, `xherd_handoff_written`, `xherd_issue_add`, `xherd_memory_propose`. Arguments mirror the CLI flags; refusals are tool errors with the same codes and messages. Status reporting stays on hooks/`report`.

## 8. HTTP

The CLI and MCP server call `POST /api/v1/xap` with `Authorization: Bearer $XHERD_AGENT_TOKEN`:

```json
{ "id": "01J…", "agentId": "…", "cmd": "gate.open", "args": { "stage": "spec", "role": "spec", "docs": ["spec.md"] }, "at": 1762450000000 }
```

Response: `200 {"ok": true, "result": {...}}` or `422 {"ok": false, "code": "doc_schema", "message": "spec.md: frontmatter is missing ticket"}`. Status reports use `POST /api/v1/hooks` with `{id, agentId, event, payload, seq, at}`.

## 9. Versioning

This is protocol version **1**. Packs declare `protocol: 1`. New optional flags and commands are minor changes. Removing or changing the meaning of a command, flag or refusal code is a new major; the daemon then supports the current and previous major.
