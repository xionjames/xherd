# xherd

Run, watch and steer a herd of coding agents working on your tickets in parallel.

## Why xherd exists

I got overwhelmed running several coding tasks in parallel. Terminal windows piled up, virtual desktops multiplied, and I kept losing track of which agent was doing what. Worse, an agent would sit for an hour waiting for an answer I never saw.

I tried a number of good existing tools. Each one gave me one or two things I liked for my daily work, but none gave me everything I needed. So I decided to build the one I wanted, and to vibe code it with the same kind of agents it now manages.

xherd is that tool. It is opinionated and built around my own workflow, and I hope it is useful to yours too.

## What xherd is

xherd is a small daemon with a web UI. It manages a herd of coding agents so you don't have to juggle them by hand.

- **One ticket, one sandbox.** Every task gets its own git worktree and its own tmux session, so agents never step on each other or on your checkout.
- **One screen for all of them.** The browser shows every agent, what it is doing, and which ones are waiting for you. Nothing sits unanswered in a forgotten terminal.
- **You stay in control.** Agents follow a spec-driven flow (spec, design, tasks, commits, pull request) and stop at approval gates. You read what they propose, then approve or ask for changes.
- **Bring your own agent.** Claude Code today. The runtime, the flow and the integrations are plugins, and your own spec-driven process can plug in too.
- **Run it in Docker or natively.** The Docker setup keeps agents and toolchains in a container and shares only your workspace, and you can still open the code in your IDE.

In short: you describe the work, agents do it in parallel, and xherd makes sure you always know where each one stands and what needs your decision.

> **Status: early, but it runs.** One agent works end to end (worktree, Claude Code in tmux, approval gates, commits, draft PR), with a web UI and a Docker setup. It is a single-user tool for now, and the planned items below are not built yet. See the [roadmap](ROADMAP.md) and open an issue with feedback.

## Highlights

Working today:

- Many agents in parallel, each isolated in its own git worktree and tmux session, with a concurrency cap.
- A web UI: the herd at a glance, a "your call" band for whatever needs you, a live terminal per agent, diffs, and settings with setup checks.
- Approval gates for spec, design, tasks and commits. Approvals are hashed, so you approve exactly what you read, and each step starts a fresh session that carries over a short `memory.md`.
- xherd runs the approved commit for you, and the agent opens a draft pull request when you approve the ship gate.
- Pluggable spec-driven development: **xherd** built in, or your own through flow packs or a few `xherd` calls ([guide](doc/integrate-your-sdd.md)).
- Docker-first: agents, toolchains (Java, Node, Go, Rust, Python) and Claude Code live in one container and only your workspace is shared. "Open in" your IDE through same-path mounts.
- Agents survive restarts: they are parked at their gate or resumed, not lost.
- Git access from the UI (HTTPS tokens, SSH keys, per-owner commit identity) and repositories cloned by URL.
- Plugins in any language (the CLI is the plugin API). Single Rust binary, web UI embedded.

Planned:

- Adjustable autonomy, tripwires and guardrails (for example, blocking pushes).
- Following the pull request after it opens: CI, review comments, replies.
- Other runtimes beyond Claude Code (Codex, opencode…), and a runtime, model and effort per step.
- Board, review and time views, notifications, and access from your phone.
- Declarative views and backups.

## Documentation

- [Getting started](doc/getting-started.md): run it (Docker recommended), first agent, everyday commands
- [Docker](doc/docker.md): topology, the two-user container, git access, toolchains, operating
- [Architecture](doc/architecture.md): crates, actors, gates, sessions, plugins, guardrails
- [CLI reference](doc/cli.md) and [Configuration](doc/configuration.md)
- [Security model](doc/security.md)
- [Plugins and flow packs](doc/plugins-and-packs.md): extending xherd
- [Development](doc/development.md): build, test, conventions, how to add things
- [XAP reference](doc/xap.md): the protocol agents and SDDs use to talk to xherd
- [Add xherd support to your SDD](doc/integrate-your-sdd.md)
- [Roadmap](ROADMAP.md)

Working documents, to be removed when the final documentation is complete: [Product specification](doc/spec.md) and [Design](doc/design.md).

## License

[MIT](LICENSE)
