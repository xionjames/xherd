# Roadmap

High-level view of what is left. The sources of truth stay where they were:
the milestone checklist in [`doc/design.md`](doc/design.md) §18, the roadmap in
[`doc/spec.md`](doc/spec.md) §21 and the status notes in [`AGENTS.md`](AGENTS.md).
This file groups the pending work by theme; it does not replace them.

## Finish what is started (M2 UI, M3 Docker-first)

- **UI screens still hidden:** Board, Review, TODOs, Time and the notification bell.
- **Settings:** live reload of the daemon config (most settings apply at the next start); plugin secrets and plugin install from the UI.
- **Docker/Colima:** SSH agent forwarding for Colima, openvscode-server (R-705), `xherd login`.
- **Git access:** block an agent on git auth failures (R-688).
- **Toolchains:** profile `toolchain:` / `setup:` (R-654, needs profiles) and the Welcome/Settings toolchain pickers (R-653).

## Agents that survive and recover

- **Plugins switched off live:** the toolchain list, the host plugin and the trust writer are still built at daemon start.
- **Pause with a handoff** (R-026): pause currently just ends the session.
- **Background-task guard** before a fresh session (R-024) and **runtime change between steps** (R-025).
- **Recovery beyond a lost session:** a failed start stays in `error` (no auto-retry); optionally a "diagnosing" session that is told what failed.
- **Check the Resume button on `parked` agents:** the lifecycle table has no Resume transition from `parked` (approving is the way out).
- **Per-agent permission mode:** `ask` / `edits` / `auto` / `all` are defined, but new agents always use `edits`.

## Context and flow quality

- **`memory.md` code map:** nothing checks its size or accuracy; measure the token savings per step and consider validating it.
- **Packs:** JSON schemas for gate documents (R-204), `views`, `extends` and `parse` for external packs.
- **More flows:** PR-review flow pack, research/design flow pack, multi-repo tickets.
- **Memory:** `xherd memory propose` and the memory panel.

## Git, accounts and credentials

- **Second account on the same host:** commit identity can follow the repository owner today, but credentials are still one per host; pushing as the work account needs a token per owner.
- **Identity helper:** a "Use my GitHub account" button that fills name and email from `gh`.
- **Native installs:** the managed gitconfig must be included from `~/.gitconfig` by the user.

## M4 → v1.0

- **Guardrails:** the daemon/agent user split (single user in the container today), the pre-push guard (R-1204), code tripwires on commits (R-115…R-117, secret tripwire), `diff_viewed` marks.
- **Backups.**
- **Autonomy `important`** and the ship stage's auto-approved variant.
- **PR follow-up:** PR polling, comments triage and replies, CI status in the UI, pause/resume on new comments.
- **Notifications:** browser, desktop, ntfy; phones.

## v1.x

- Codex and opencode runtimes.
- Kanban, To review and Work log views.
- Toolchain plugins and the Welcome installer; plugin install from the UI with a catalogue.
- TODOs, budgets, `never` autonomy.

## Later

- ACP driver per runtime, selectable per agent.
- Server / Kubernetes deployment; VS Code Remote-SSH entries.
- Electron desktop app (tray, notifications, built-in host companion).
- Panel plugins; per-agent containers with egress allowlists; parallel task execution inside one agent.
- `xherd mcp`.

## Verify before relying on it

Things this work assumed and could not confirm:

- `claude auth status` exits non-zero when logged out (only run logged in).
- Claude Code first-run flags in the container's `~/.claude.json`, and whether `claude --resume` finds its transcript after a container recreate (assumes the home volume persists).
- The new UI layout (one-row band, vertical menu) in a browser at several widths.
- A full agent run (spec → ship) with the plain-`xherd` prompts, the allow rules and the `edits` default.
