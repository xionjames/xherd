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

> **Status: design phase.** Nothing is runnable yet. Read the docs below and open an issue with feedback.

## Highlights (planned)

- Many agents in parallel, each isolated in a worktree, with a concurrency cap.
- Approval gates for spec, design, tasks, commits and PR replies, with adjustable autonomy and tripwires.
- Pluggable spec-driven development: **xhSpec** built in; your own SDD through flow packs or a few `xherd` calls ([guide](doc/integrate-your-sdd.md)).
- Docker-first, with "Open in" your IDE through same-path workspace mounts; native binary too.
- A runtime, model and effort per step; live terminal or ACP driver.
- Plugins in any language (the CLI is the plugin API), declarative views, guardrails, backups.
- Single Rust binary, web UI embedded.

## Documentation

- [Product specification](doc/spec.md): what xherd does, MVP scope, guardrails, updates, backups
- [Design](doc/design.md): architecture, Rust stack, Docker setup, implementation notes
- [XAP reference](doc/xap.md): the protocol agents and SDDs use to talk to xherd
- [Add xherd support to your SDD](doc/integrate-your-sdd.md)

## License

[MIT](LICENSE)
