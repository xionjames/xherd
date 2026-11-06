# xherd

Run, watch and steer a herd of coding agents working on your tickets in parallel.

xherd gives each ticket its own git worktree and terminal session and starts a coding agent (Claude Code, Codex, opencode…) with a spec-driven flow. It stops the agent at approval gates and lets you approve or ask for changes from your browser or phone, then follows the pull request until it is merged.

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
