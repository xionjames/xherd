<p align="center">
  <img src="doc/assets/logo.svg" alt="xherd logo: four agents converging on one watching eye" width="96" height="96">
</p>

<h1 align="center">xherd</h1>

<p align="center"><b>Run a herd of coding agents in parallel. Stay in control.</b></p>

> [!WARNING]
> **Early releases (0.x).** xherd is under active development. Expect breaking changes between versions, missing features and rough edges, and back up anything you care about (`xherd backup create`) before updating.

## Why

I got overwhelmed running several coding tasks at once: terminals piled up and an agent could wait an hour for an answer I never saw. The tools I tried each solved part of it, so I built the one I wanted, with the same kind of agents it now manages. It is opinionated and built around my workflow; I hope it fits yours.

## What is xherd?

https://github.com/user-attachments/assets/d9fe9a6b-00df-4fe2-a009-7a0889b572e9

*A short tour, with mocked data: the Herd, agents by stage, an agent's panels, a design with a diagram, an inline comment on a diff, approving a commit, the Board, Needs attention and the plugins. More screens and the CLI: [the tour](doc/user/tour.md).*

You hand xherd your tickets. It gives each one to a coding agent in its own sandbox, and shows you all of them on one screen.

- **One ticket, one sandbox.** Each task gets its own git worktree and tmux session, so agents never step on each other or on your checkout.
- **One screen for the herd.** The browser shows what every agent is doing and which ones are waiting for you.
- **You approve the important steps.** Agents follow a spec-driven flow (spec, design, tasks, commits, pull request) and stop at gates. You read what they propose, then approve or ask for changes.
- **Bring your own agent and process.** Claude Code, opencode or Codex; the runtime, the flow and the integrations are plugins.
- **Docker or native.** Docker keeps agents and toolchains in a container and shares only your workspace; you can still open the code in your IDE.

![The Herd: two agents waiting for you on top, every agent below with its status, progress and tasks](doc/assets/herd.jpg)

> **Status: early, but it runs.** Agents work end to end (worktree, runtime in tmux, approval gates, commits, draft PR, PR follow-up), with a web UI, a Docker image and Linux packages ([releases](https://github.com/xionjames/xherd/releases)). It is a single-user tool for now, and the planned items below are not built yet. See the [roadmap](ROADMAP.md) and open an issue with feedback.

## Features

### Agents

- Many agents in parallel, each in its own git worktree, branch and tmux session, with a concurrency cap.
- Claude Code, opencode or Codex, chosen per agent, with a model per agent and a tool-permission mode (ask, edits, auto, all).
- Named agents with an optional ticket (an issue URL or key) that prefixes the branch; repositories by path or cloned by URL.
- A live terminal per agent in the browser. **Ask** gives a stopped agent a quick session with your question, held until you finish it.
- Pause with a handoff note, resume where the conversation left off, and "Pause all" as a kill switch.
- Agents survive restarts: parked at their gate or resumed, not lost.

### Review and approval

- A spec-driven flow built in (spec, design, tasks, implement, review, ship), defined as a flow pack whose prompts you can edit.
- Every step stops at a gate. Documents render Markdown with Mermaid diagrams; comment on any block or diff line and send it all back as one change request.
- Approvals are hashed, so you approve exactly what you read. xherd makes the approved commit itself.
- Each step starts a fresh session that carries over a short `memory.md`; the agent can propose updates to the repo's shared `AGENTS.md`.

### Pull requests

- Approving the ship gate has the agent push and open a draft PR. xherd itself never pushes.
- The PR's checks, conflicts and reviewers are polled, or checked at once with *Check now*.
- Review comments become a round: the agent proposes fix, reply or skip per comment, you decide, replies are posted only after you approve.
- **Resolve conflicts** has the agent merge the base branch (never a rebase), and the merge commit always waits for you.
- GitHub and GitLab for pull requests; Bitbucket for git access only.

### Autonomy and guardrails

- Adjustable autonomy: xherd can approve routine steps itself, and lists them for you to look over later.
- Tripwires hold a step for you: secrets in the diff, sensitive paths, large diffs, files outside the task's scope, failing lint or tests.
- A pre-push guard, token budgets per agent and per day, a session time limit, a cap on automatic approvals in a row and on review rounds.
- In Docker, the daemon and the agents run as different users, so agents cannot read xherd's secrets.

### Your view of the work

- The Herd (as a list, by stage, or a view plugin of your own), a Board of every stage and task, and **Needs attention** with what waits for you.
- Notifications in the bell, the browser, the desktop (native installs) and on your phone through ntfy.
- Issue trackers (Jira, GitHub issues, GitLab issues): ticket keys become links, the ticket shows beside the agent, and you can post a summary once the PR is open.
- Time tracking per ticket, and TODOs: reminders, or conditions a read-only agent run checks for you.

### Setup and operation

- Docker-first: agents, toolchains (Java, Node, Go, Rust, Python) and the agent CLIs live in one container; only your workspace is shared. Open the worktree in your IDE or your own terminal.
- Git access from the UI (HTTPS tokens, SSH keys, commit identity per repository owner) with a doctor that says what is wrong.
- Repo profiles: base branch, autonomy, tripwires, test and lint commands, instructions and defaults per repository.
- Backups of agents and their work (`xherd backup create|verify|restore`), taken before every `xherd docker update`.
- Plugins in any language (the CLI is the plugin API). One Rust binary with the web UI embedded; most actions are also CLI commands.

### Planned

- A runtime, model and effort per step.
- Secure access from your phone or another device (pairing), beyond notifications.
- Board cards and views declared by flow packs and profiles.
- Your own spec-driven method under xherd, through your own flow pack or a few `xherd` calls ([draft guide](doc/extend/integrate-your-sdd.md)).

## Documentation

Start with **[Getting started](doc/user/getting-started.md)** (install with Docker or a Linux package, first agent), then **[Using xherd](doc/user/guide.md)**. Everything else is in the [documentation index](doc/README.md):

| | |
|---|---|
| Running it | [Getting started](doc/user/getting-started.md) · [Tour](doc/user/tour.md) · [Using xherd](doc/user/guide.md) · [Docker](doc/user/docker.md) · [Security model](doc/user/security.md) |
| Reference | [CLI](doc/reference/cli.md) · [Configuration](doc/reference/configuration.md) · [HTTP API](doc/reference/http-api.md) · [XAP](doc/reference/xap.md) |
| Extending | [Plugins](doc/extend/plugins.md) · [Flow packs](doc/extend/flow-packs.md) · [Add xherd support to your SDD](doc/extend/integrate-your-sdd.md) |
| Contributing | [Running from source](doc/dev/from-source.md) · [Development](doc/dev/development.md) · [Architecture](doc/dev/architecture.md) · [Roadmap](ROADMAP.md) |

## License

[GPL-3.0-or-later](LICENSE)
